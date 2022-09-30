use crate::tensor::{Tensor, ValTensor, VarTensor};

use crate::tensor::TensorType;
use anyhow::Result;
use halo2_proofs::{
    arithmetic::FieldExt,
    circuit::{Layouter, SimpleFloorPlanner, Value},
    plonk::{Circuit, ConstraintSystem, Error},
};
use std::marker::PhantomData;

pub mod utilities;
pub use utilities::*;

pub mod onnxmodel;
pub use onnxmodel::*;

#[derive(Clone, Debug)]
pub struct OnnxCircuit<F: FieldExt, const BITS: usize> {
    pub input: Tensor<i32>,
    pub _marker: PhantomData<F>,
}

impl<F: FieldExt + TensorType, const BITS: usize> Circuit<F> for OnnxCircuit<F, BITS> {
    type Config = OnnxModelConfig<F, BITS>;
    type FloorPlanner = SimpleFloorPlanner;

    fn without_witnesses(&self) -> Self {
        self.clone()
    }

    fn configure(meta: &mut ConstraintSystem<F>) -> Self::Config {
        let mut onnx_model = OnnxModel::from_arg();
        let num_advices = onnx_model.max_advices_width().unwrap();
        let num_fixeds = onnx_model.max_fixeds_width().unwrap();
        let advices = VarTensor::from(Tensor::from((0..num_advices + 3).map(|_| {
            let col = meta.advice_column();
            meta.enable_equality(col);
            col
        })));
        let fixeds = VarTensor::from(Tensor::from((0..num_fixeds + 3).map(|_| {
            let col = meta.fixed_column();
            meta.enable_equality(col);
            col
        })));

        onnx_model.configure(meta, advices, fixeds).unwrap()
    }

    fn synthesize(
        &self,
        config: Self::Config,
        mut layouter: impl Layouter<F>,
    ) -> Result<(), Error> {
        let onnx_model = OnnxModel::from_arg();
        let input = ValTensor::from(<Tensor<i32> as Into<Tensor<Value<F>>>>::into(
            self.input.clone(),
        ));
        // let input: Tensor<F> = self.input.clone().into();
        // let input = ValTensor::from(input);
        let output = onnx_model
            .layout(config.clone(), &mut layouter, input)
            .unwrap();

        match output {
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
