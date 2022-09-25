use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use halo2_proofs::dev::MockProver;
use halo2_proofs::{
    arithmetic::FieldExt,
    circuit::{Layouter, Value, SimpleFloorPlanner},
    plonk::{Circuit, ConstraintSystem, Error},
};
use halo2curves::pasta::Fp as F;
use halo2deeplearning::tensor::*;
use halo2deeplearning::tensor_ops::eltwise::{EltwiseConfig, Nonlin1d, Nonlinearity, ReLu};
use rand::Rng;
use std::marker::PhantomData;

const BITS: usize = 8;
static mut LEN: usize = 4;

#[derive(Clone)]
struct NLCircuit<F: FieldExt + TensorType, NL: Nonlinearity<F>> {
    assigned: Nonlin1d<F, NL>,
    _marker: PhantomData<NL>,
}

impl<F: FieldExt + TensorType, NL: 'static + Nonlinearity<F> + Clone> Circuit<F>
    for NLCircuit<F, NL>
{
    type Config = EltwiseConfig<F, BITS, NL>;
    type FloorPlanner = SimpleFloorPlanner;

    fn without_witnesses(&self) -> Self {
        self.clone()
    }

    fn configure(cs: &mut ConstraintSystem<F>) -> Self::Config {
        unsafe {
            let advices = VarTensor::Advice {
                inner: (0..LEN).map(|_| cs.advice_column()).into(),
                dims: [LEN].to_vec(),
            };
            Self::Config::configure(cs, advices, None)
        }
    }

    fn synthesize(
        &self,
        config: Self::Config,
        mut layouter: impl Layouter<F>, // layouter is our 'write buffer' for the circuit
    ) -> Result<(), Error> {
        config.layout(&mut layouter, self.assigned.input.clone());

        Ok(())
    }
}

fn runrelu(c: &mut Criterion) {
    let mut group = c.benchmark_group("relu");
    for size in [1, 2, 4, 8, 16, 32].iter() {
        let len = unsafe {
            LEN = size * 4;
            LEN
        };
        group.throughput(Throughput::Elements(len as u64));
        group.bench_with_input(BenchmarkId::from_parameter(len), &len, |b, &_| {
            b.iter(|| {
                let k = 9; //2^k rows
                           // parameters
                let mut rng = rand::thread_rng();

                let input = (0..len).map(|_| rng.gen_range(0..10)).collect::<Vec<_>>();
                // input data, with 1 padding to allow for bias
                let input = Tensor::<i32>::new(Some(&input), &[len]).unwrap();

                let relu_v: Tensor<Value<F>> = input.into();
                let assigned: Nonlin1d<F, ReLu<F>> = Nonlin1d {
                    input: ValTensor::from(relu_v.clone()),
                    output: ValTensor::from(relu_v),
                    _marker: PhantomData,
                };

                let circuit = NLCircuit::<F, ReLu<F>> {
                    assigned,
                    _marker: PhantomData,
                };

                let prover = MockProver::run(k, &circuit, vec![]).unwrap();
                prover.assert_satisfied();
            });
        });
    }
    group.finish();
}

criterion_group!(benches, runrelu);
criterion_main!(benches);
