use halo2_proofs::dev::MockProver;
use halo2_proofs::{
    arithmetic::FieldExt,
    circuit::{Layouter, SimpleFloorPlanner},
    plonk::{Circuit, Column, ConstraintSystem, Error, Instance},
};
use halo2curves::pasta::Fp as F;
use halo2deeplearning::fieldutils::i32tofelt;
use halo2deeplearning::nn::affine::Affine1dConfig;
use halo2deeplearning::nn::*;
use halo2deeplearning::tensor::{Tensor, TensorType};
use halo2deeplearning::tensor_ops::eltwise::{DivideBy, EltwiseConfig, EltwiseTable, ReLu};
use std::marker::PhantomData;
use std::{cell::RefCell, rc::Rc};

// A columnar ReLu MLP
#[derive(Clone)]
struct MyConfig<
    F: FieldExt + TensorType,
    const LEN: usize, //LEN = CHOUT x OH x OW flattened //not supported yet in rust
    const BITS: usize,
> {
    l0: Affine1dConfig<F, LEN, LEN>,
    l1: EltwiseConfig<F, BITS, ReLu<F>>,
    l2: Affine1dConfig<F, LEN, LEN>,
    l3: EltwiseConfig<F, BITS, ReLu<F>>,
    l4: EltwiseConfig<F, BITS, DivideBy<F, 128>>,
    public_output: Column<Instance>,
}

#[derive(Clone)]
struct MyCircuit<
    F: FieldExt,
    const LEN: usize, //LEN = CHOUT x OH x OW flattened
    const BITS: usize,
> {
    // Given the stateless MyConfig type information, a DNN trace is determined by its input and the parameters of its layers.
    // Computing the trace still requires a forward pass. The intermediate activations are stored only by the layouter.
    input: Tensor<i32>,
    l0_params: [Tensor<i32>; 2],
    l2_params: [Tensor<i32>; 2],
    _marker: PhantomData<F>,
}

impl<F: FieldExt + TensorType, const LEN: usize, const BITS: usize> Circuit<F>
    for MyCircuit<F, LEN, BITS>
{
    type Config = MyConfig<F, LEN, BITS>;
    type FloorPlanner = SimpleFloorPlanner;

    fn without_witnesses(&self) -> Self {
        self.clone()
    }

    // Here we wire together the layers by using the output advice in each layer as input advice in the next (not with copying / equality).
    // This can be automated but we will sometimes want skip connections, etc. so we need the flexibility.
    fn configure(cs: &mut ConstraintSystem<F>) -> Self::Config {
        let advices = ParamType::Advice(Tensor::from((0..LEN + 3).map(|_| {
            let col = cs.advice_column();
            cs.enable_equality(col);
            col
        })));

        let relutable_config = EltwiseTable::<F, BITS, ReLu<F>>::configure(cs);

        let relutable = Rc::new(RefCell::new(relutable_config));

        let kernel = advices.get_slice(&[0..LEN]);
        let bias = advices.get_slice(&[LEN + 2..LEN + 3]);

        let l0 = Affine1dConfig::<F, LEN, LEN>::configure(
            cs,
            &[kernel.clone(), bias.clone()],
            advices.get_slice(&[LEN..LEN + 1]),
            advices.get_slice(&[LEN + 1..LEN + 2]),
        );

        let l1: EltwiseConfig<F, BITS, ReLu<F>> =
            EltwiseConfig::configure(cs, advices.get_slice(&[0..LEN]), Some(relutable.clone()));

        let l2 = Affine1dConfig::<F, LEN, LEN>::configure(
            cs,
            &[kernel, bias],
            advices.get_slice(&[LEN..LEN + 1]),
            advices.get_slice(&[LEN + 1..LEN + 2]),
        );

        let l3: EltwiseConfig<F, BITS, ReLu<F>> =
            EltwiseConfig::configure(cs, advices.get_slice(&[0..LEN]), Some(relutable.clone()));

        let l4: EltwiseConfig<F, BITS, DivideBy<F, 128>> =
            EltwiseConfig::configure(cs, advices.get_slice(&[0..LEN]), None);

        let public_output: Column<Instance> = cs.instance_column();
        cs.enable_equality(public_output);

        MyConfig {
            l0,
            l1,
            l2,
            l3,
            l4,
            public_output,
        }
    }

    fn synthesize(
        &self,
        config: Self::Config,
        mut layouter: impl Layouter<F>,
    ) -> Result<(), Error> {
        let x = self.input.clone();
        let x = config.l0.layout(
            &mut layouter,
            IOType::Value(x.into()),
            &self
                .l0_params
                .iter()
                .map(|a| IOType::Value(a.clone().into()))
                .collect::<Vec<IOType<F>>>(),
        );
        let x = config.l1.layout(&mut layouter, x);
        let x = config.l2.layout(
            &mut layouter,
            x,
            &self
                .l2_params
                .iter()
                .map(|a| IOType::Value(a.clone().into()))
                .collect::<Vec<IOType<F>>>(),
        );
        let x = config.l3.layout(&mut layouter, x);
        let x = config.l4.layout(&mut layouter, x);
        match x {
            IOType::PrevAssigned(v) => v.enum_map(|i, x| {
                layouter
                    .constrain_instance(x.cell(), config.public_output, i)
                    .unwrap()
            }),
            _ => panic!("Should be assigned"),
        };
        Ok(())
    }
}

pub fn runmlp() {
    let k = 15; //2^k rows
                // parameters
    let l0_kernel = Tensor::<i32>::new(
        Some(&[10, 0, 0, -1, 0, 10, 1, 0, 0, 1, 10, 0, 1, 0, 0, 10]),
        &[4, 4],
    )
    .unwrap();
    let l0_bias = Tensor::<i32>::new(Some(&[0, 0, 0, 1]), &[1, 4]).unwrap();

    let l2_kernel = Tensor::<i32>::new(
        Some(&[0, 3, 10, -1, 0, 10, 1, 0, 0, 1, 0, 12, 1, -2, 32, 0]),
        &[4, 4],
    )
    .unwrap();
    // input data, with 1 padding to allow for bias
    let input = Tensor::<i32>::new(Some(&[-30, -21, 11, 40]), &[1, 4]).unwrap();
    let l2_bias = Tensor::<i32>::new(Some(&[0, 0, 0, 1]), &[1, 4]).unwrap();

    let circuit = MyCircuit::<F, 4, 14> {
        input,
        l0_params: [l0_kernel, l0_bias],
        l2_params: [l2_kernel, l2_bias],
        _marker: PhantomData,
    };

    let public_input: Vec<i32> = unsafe {
        vec![
            (531f32 / 128f32).round().to_int_unchecked::<i32>().into(),
            (103f32 / 128f32).round().to_int_unchecked::<i32>().into(),
            (4469f32 / 128f32).round().to_int_unchecked::<i32>().into(),
            (2849f32 / 128f32).to_int_unchecked::<i32>().into(),
        ]
    };

    println!("public input {:?}", public_input);

    let prover = MockProver::run(
        k,
        &circuit,
        vec![public_input
            .iter()
            .map(|x| i32tofelt::<F>(*x).into())
            .collect()],
        //            vec![vec![(4).into(), (1).into(), (35).into(), (22).into()]],
    )
    .unwrap();
    prover.assert_satisfied();
}

pub fn main() {
    runmlp()
}
