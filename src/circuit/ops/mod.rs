use std::error::Error;

use halo2_proofs::circuit::Region;
use halo2curves::FieldExt;
use serde::Serialize;

use crate::tensor::{Tensor, TensorError, TensorType, ValTensor};

use self::{lookup::LookupOp, poly::PolyOp};

///
pub mod base;
///
pub mod hybrid;
/// Layouts for specific functions (composed of base ops)
pub mod layouts;
///
pub mod lookup;
///
pub mod poly;

///
pub trait Op<F: FieldExt + TensorType>: std::fmt::Debug + Send + Sync {
    ///
    fn f(&self, x: &[Tensor<i128>]) -> Result<Tensor<i128>, TensorError>;
    ///
    fn as_str(&self) -> &'static str;

    ///
    fn layout(
        &self,
        config: &mut crate::circuit::BaseConfig<F>,
        region: Option<&mut Region<F>>,
        values: &[ValTensor<F>],
        offset: &mut usize,
    ) -> Result<Option<ValTensor<F>>, Box<dyn Error>>;

    ///
    fn out_scale(&self, _: Vec<u32>, global_scale: u32) -> u32 {
        global_scale
    }

    ///
    fn has_3d_input(&self) -> bool {
        false
    }

    ///
    fn requires_homogenous_input_scales(&self) -> bool {
        false
    }

    ///
    fn required_poly(&self) -> Option<PolyOp<F>> {
        None
    }

    ///
    fn required_lookup(&self) -> Option<LookupOp> {
        None
    }

    ///
    fn rescale(&self, inputs_scale: Vec<u32>, global_scale: u32) -> Box<dyn Op<F>>;

    ///
    fn is_input(&self) -> bool {
        false
    }

    ///
    fn clone_dyn(&self) -> Box<dyn Op<F>>;
}

impl<F: FieldExt + TensorType> Clone for Box<dyn Op<F>> {
    fn clone(&self) -> Self {
        self.clone_dyn()
    }
}

///
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct Input;

impl<F: FieldExt + TensorType> Op<F> for Input {
    fn f(&self, x: &[Tensor<i128>]) -> Result<Tensor<i128>, TensorError> {
        Ok(x[0].clone())
    }

    fn as_str(&self) -> &'static str {
        "Input"
    }
    fn layout(
        &self,
        _: &mut crate::circuit::BaseConfig<F>,
        _: Option<&mut Region<F>>,
        _: &[ValTensor<F>],
        _: &mut usize,
    ) -> Result<Option<ValTensor<F>>, Box<dyn Error>> {
        Ok(None)
    }

    fn rescale(&self, _: Vec<u32>, _: u32) -> Box<dyn Op<F>> {
        Box::new(self.clone())
    }

    fn is_input(&self) -> bool {
        true
    }

    fn clone_dyn(&self) -> Box<dyn Op<F>> {
        Box::new(self.clone()) // Forward to the derive(Clone) impl
    }
}

///
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct Unknown;

impl<F: FieldExt + TensorType> Op<F> for Unknown {
    fn f(&self, _: &[Tensor<i128>]) -> Result<Tensor<i128>, TensorError> {
        Err(TensorError::WrongMethod)
    }

    fn as_str(&self) -> &'static str {
        "Unknown"
    }
    fn layout(
        &self,
        _: &mut crate::circuit::BaseConfig<F>,
        _: Option<&mut Region<F>>,
        _: &[ValTensor<F>],
        _: &mut usize,
    ) -> Result<Option<ValTensor<F>>, Box<dyn Error>> {
        Err(Box::new(super::CircuitError::UnsupportedOp))
    }
    fn rescale(&self, _: Vec<u32>, _: u32) -> Box<dyn Op<F>> {
        Box::new(self.clone())
    }

    fn clone_dyn(&self) -> Box<dyn Op<F>> {
        Box::new(self.clone()) // Forward to the derive(Clone) impl
    }
}
