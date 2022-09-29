#[cfg(feature = "onnx")]
mod onnx_example {
    use halo2_proofs::dev::MockProver;
    use halo2_proofs::{
        arithmetic::FieldExt,
        circuit::{Layouter, SimpleFloorPlanner, Value},
        plonk::{Circuit, Column, ConstraintSystem, Error, Instance},
    };
    use halo2curves::pasta::Fp as F;
    use halo2deeplearning::fieldutils::i32_to_felt;
    use halo2deeplearning::nn::affine::Affine1dConfig;
    use halo2deeplearning::nn::*;
    use halo2deeplearning::onnx::OnnxModel;
    use halo2deeplearning::tensor::{Tensor, TensorType, ValTensor, VarTensor};
    use halo2deeplearning::nn::eltwise::{EltwiseConfig, ReLu};
    use std::marker::PhantomData;

    #[derive(Clone)]
    struct MyConfig<F: FieldExt + TensorType, const BITS: usize> {
        l0: Affine1dConfig<F>,
        l1: EltwiseConfig<F, BITS, ReLu<F>>,
        public_output: Column<Instance>,
    }

    #[derive(Clone)]
    struct MyCircuit<F: FieldExt + TensorType, const BITS: usize> {
        input: ValTensor<F>,
        l0_params: [ValTensor<F>; 2],
        _marker: PhantomData<F>,
    }

    impl<F: FieldExt + TensorType, const BITS: usize> Circuit<F> for MyCircuit<F, BITS> {
        type Config = MyConfig<F, BITS>;
        type FloorPlanner = SimpleFloorPlanner;

        fn without_witnesses(&self) -> Self {
            self.clone()
        }

        fn configure(cs: &mut ConstraintSystem<F>) -> Self::Config {
            let onnx_model = OnnxModel::new("examples/onnx_models/ff.onnx");
            let l0_kernel = onnx_model.get_tensor_by_node_name("fc1.weight", 0f32, 256f32);
            let shape = l0_kernel.dims();
            let (out_dims, in_dims) = (shape[0], shape[1]);

            let advices = VarTensor::from(Tensor::from((0..out_dims + 3).map(|_| {
                let col = cs.advice_column();
                cs.enable_equality(col);
                col
            })));

            let kernel = advices.get_slice(&[0..out_dims], &[out_dims, in_dims]);
            let bias = advices.get_slice(&[out_dims + 2..out_dims + 3], &[out_dims]);

            let l0 = Affine1dConfig::<F>::configure(
                cs,
                &[
                    kernel,
                    bias,
                    advices.get_slice(&[out_dims..out_dims + 1], &[in_dims]),
                    advices.get_slice(&[out_dims + 1..out_dims + 2], &[out_dims]),
                ],
            );

            let l1: EltwiseConfig<F, BITS, ReLu<F>> =
                EltwiseConfig::configure(cs, &[advices.get_slice(&[0..out_dims], &[out_dims])]);

            let public_output: Column<Instance> = cs.instance_column();
            cs.enable_equality(public_output);

            MyConfig {
                l0,
                l1,
                public_output,
            }
        }

        fn synthesize(
            &self,
            config: Self::Config,
            mut layouter: impl Layouter<F>,
        ) -> Result<(), Error> {
            let x = config.l0.layout(
                &mut layouter,
                &[
                    self.input.clone(),
                    self.l0_params[0].clone(),
                    self.l0_params[1].clone(),
                ],
            );

            let x = config.l1.layout(&mut layouter, &[x]);

            match x {
                ValTensor::PrevAssigned { inner: v, dims: _ } => v.enum_map(|i, x| {
                    layouter
                        .constrain_instance(x.cell(), config.public_output, i)
                        .unwrap()
                }),
                _ => panic!("Should be assigned"),
            };
            Ok(())
        }
    }

    pub fn run() {
        let k = 15; //2^k rows

        let onnx_model = OnnxModel::new("examples/onnx_models/ff.onnx");

        let l0_kernel: Tensor<Value<F>> = onnx_model
            .get_tensor_by_node_name("fc1.weight", 0f32, 256f32)
            .into();
        let l0_bias: Tensor<Value<F>> = onnx_model
            .get_tensor_by_node_name("fc1.bias", 0f32, 256f32)
            .into();

        let input: Tensor<Value<F>> = Tensor::<i32>::new(Some(&[-30, -21, 11]), &[3])
            .unwrap()
            .into();

        let circuit = MyCircuit::<F, 14> {
            input: input.into(),
            l0_params: [l0_kernel.into(), l0_bias.into()],
            _marker: PhantomData,
        };

        let public_input: Vec<i32> = vec![0, 0, 0, 1653];

        println!("public input {:?}", public_input);

        let prover = MockProver::run(
            k,
            &circuit,
            vec![public_input.iter().map(|x| i32_to_felt::<F>(*x)).collect()],
        )
        .unwrap();
        prover.assert_satisfied();
    }
}
#[cfg(feature = "onnx")]
pub fn main() {
    crate::onnx_example::run()
}
#[cfg(not(feature = "onnx"))]
pub fn main() {}
