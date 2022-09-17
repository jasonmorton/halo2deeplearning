use super::*;
use crate::nn::io::*;
use crate::tensor::{Tensor, TensorType};
use halo2_proofs::{
    arithmetic::FieldExt,
    circuit::{AssignedCell, Layouter, Value},
    plonk::{Assigned, ConstraintSystem, Constraints, Expression, Selector},
};
use std::marker::PhantomData;

#[derive(Clone)]
pub struct Affine1dConfig<F: FieldExt + TensorType, const IN: usize, const OUT: usize> {
    // kernel is weights and biases concatenated
    pub kernel: IOConfig<F>,
    pub bias: IOConfig<F>,
    pub input: IOConfig<F>,
    pub output: IOConfig<F>,
    pub selector: Selector,
    _marker: PhantomData<F>,
}

impl<F: FieldExt + TensorType, const IN: usize, const OUT: usize> LayerConfig<F>
    for Affine1dConfig<F, IN, OUT>
{
    // composable_configure takes the input tensor as an argument, and completes the advice by generating new for the rest
    fn configure(
        meta: &mut ConstraintSystem<F>,
        params: &[ParamType],
        input: ParamType,
        output: ParamType,
    ) -> Self {
        assert!(params.len() == 2);
        let (kernel, bias) = (params[0].clone(), params[1].clone());
        let config = Self {
            selector: meta.selector(),
            kernel: IOConfig::configure(meta, kernel, &[OUT, IN]),
            bias: IOConfig::configure(meta, bias, &[1, OUT]),
            // add 1 to incorporate bias !
            input: IOConfig::configure(meta, input, &[1, IN]),
            output: IOConfig::configure(meta, output, &[1, OUT]),
            _marker: PhantomData,
        };

        meta.create_gate("affine", |meta| {
            let selector = meta.query_selector(config.selector);
            // Get output expressions for each input channel
            let expected_output: Tensor<Expression<F>> = config.output.query(meta, 0);
            // Now we compute the linear expression,  and add it to constraints
            let witnessed_output = expected_output.enum_map(|i, _| {
                let mut c = Expression::Constant(<F as TensorType>::zero().unwrap());
                for j in 0..IN {
                    c = c + config.kernel.query_idx(meta, i, j) * config.input.query_idx(meta, 0, j)
                }
                c + config.bias.query_idx(meta, 0, i)
                // add the bias
            });

            let constraints = witnessed_output.enum_map(|i, o| o - expected_output[i].clone());

            Constraints::with_selector(selector, constraints)
        });

        config
    }

    fn assign(
        &self,
        layouter: &mut impl Layouter<F>,
        input: IOType<F>,
        params: &[IOType<F>],
    ) -> Tensor<AssignedCell<Assigned<F>, F>> {
        assert!(params.len() == 2);
        let (kernel, bias) = (params[0].clone(), params[1].clone());
        layouter
            .assign_region(
                || "assign image and kernel",
                |mut region| {
                    let offset = 0;
                    self.selector.enable(&mut region, offset)?;

                    let input = self.input.assign(&mut region, offset, input.clone());
                    let weights = self.kernel.assign(&mut region, offset, kernel.clone());
                    let bias = self.bias.assign(&mut region, offset, bias.clone());

                    // calculate value of output
                    let mut output: Tensor<Value<Assigned<F>>> = Tensor::new(None, &[OUT]).unwrap();
                    output = output.enum_map(|i, mut o| {
                        for (j, x) in input.iter().enumerate() {
                            o = o + x.value_field() * weights.get(&[i, j]).value_field();
                        }
                        o + bias.get(&[0, i]).value_field()
                    });

                    Ok(self
                        .output
                        .assign(&mut region, offset, IOType::AssignedValue(output)))
                },
            )
            .unwrap()
    }
    fn layout(
        &self,
        layouter: &mut impl Layouter<F>,
        input: IOType<F>,
        params: &[IOType<F>],
    ) -> IOType<F> {
        assert!(params.len() == 2);
        IOType::PrevAssigned(self.assign(layouter, input, params))
    }
}
