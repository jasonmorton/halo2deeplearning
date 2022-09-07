use halo2_proofs::{
    arithmetic::FieldExt,
    circuit::{AssignedCell, Layouter, Region, Value},
    plonk::{Advice, Assigned, Column, ConstraintSystem, Constraints, Expression, Selector},
    poly::Rotation,
};
use std::marker::PhantomData;

use crate::tensor::{Tensor, TensorType};

// We layout in two phases: first we load any parameters (returning parameters, used only in case of a tied weight model),
// then we load the input, perform the forward pass, and layout the input and output, returning the output
#[derive(Clone)]
pub struct RawParameters<const IN: usize, const OUT: usize> {
    pub weights: Tensor<i32>,
    pub biases: Tensor<i32>,
}

pub struct Parameters<F: FieldExt, const IN: usize, const OUT: usize> {
    weights: Tensor<AssignedCell<Assigned<F>, F>>,
    biases: Tensor<AssignedCell<Assigned<F>, F>>,
    pub _marker: PhantomData<F>,
}

#[derive(Clone)]
pub struct Affine1dConfig<F: FieldExt, const IN: usize, const OUT: usize>
// where
//     [(); IN + 3]:,
{
    pub weights: [Column<Advice>; IN],
    pub input: Column<Advice>,
    pub output: Column<Advice>,
    pub bias: Column<Advice>,
    pub q: Selector,
    _marker: PhantomData<F>,
}

impl<F: FieldExt + TensorType, const IN: usize, const OUT: usize> Affine1dConfig<F, IN, OUT>
// where
//     [(); IN + 3]:,
{
    pub fn layout(
        &self,
        layouter: &mut impl Layouter<F>,
        weights: Tensor<i32>,
        biases: Tensor<i32>,
        input: Tensor<AssignedCell<Assigned<F>, F>>,
    ) -> Result<Tensor<AssignedCell<Assigned<F>, F>>, halo2_proofs::plonk::Error> {
        layouter.assign_region(
            || "Both",
            |mut region| {
                let offset = 0;
                self.q.enable(&mut region, offset)?;

                let params =
                    self.assign_parameters(&mut region, offset, weights.clone(), biases.clone())?;
                let output = self.forward(&mut region, offset, input.clone(), params)?;
                Ok(output)
            },
        )
    }

    pub fn assign_parameters(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        weights: Tensor<i32>,
        biases: Tensor<i32>,
    ) -> Result<Parameters<F, IN, OUT>, halo2_proofs::plonk::Error> {
        let biases: Tensor<Value<Assigned<F>>> = biases.into();
        let weights: Tensor<Value<Assigned<F>>> = weights.into();

        let biases_for_equality = biases.enum_map(|i, b| {
            region
                .assign_advice(|| "b".to_string(), self.bias, offset + i, || b)
                .unwrap()
        });

        let weights_for_equality = weights.enum_map(|i, w| {
            region
                .assign_advice(
                    || "w".to_string(),
                    // row indices
                    self.weights[i / weights.dims()[0]],
                    // columns indices
                    offset + i % weights.dims()[1],
                    || w,
                )
                .unwrap()
        });

        let params = Parameters {
            biases: biases_for_equality,
            weights: weights_for_equality,
            _marker: PhantomData,
        };

        Ok(params)
    }

    pub fn forward(
        &self, // just advice
        region: &mut Region<'_, F>,
        offset: usize,
        input: Tensor<AssignedCell<Assigned<F>, F>>,
        params: Parameters<F, IN, OUT>,
    ) -> Result<Tensor<AssignedCell<Assigned<F>, F>>, halo2_proofs::plonk::Error> {
        // copy the input
        input.enum_map(|i, x| {
            x.copy_advice(|| "input", region, self.input, offset + i)
                .unwrap()
        });

        // calculate value of output
        let mut output: Tensor<Value<Assigned<F>>> = Tensor::new(None, &[OUT]).unwrap();
        output = output
            .enum_map(|i, mut o| {
                input.enum_map(|j, x| {
                    o = o + params.weights.get(&[i, j]).value_field() * x.value_field();
                    o + params.biases.get(&[i]).value_field()
                })
            })
            .flatten();

        // assign that value and return it
        let output_for_equality = output.enum_map(|i, o| {
            region
                .assign_advice(|| "o".to_string(), self.output, offset + i, || o)
                .unwrap()
        });

        Ok(output_for_equality)
    }

    // composable_configure takes the input tensor as an argument, and completes the advice by generating new for the rest
    pub fn configure(
        cs: &mut ConstraintSystem<F>,
        weights: [Column<Advice>; IN],
        input: Column<Advice>,
        output: Column<Advice>,
        bias: Column<Advice>,
    ) -> Self {
        let qs = cs.selector();

        cs.create_gate("affine", |virtual_cells| {
            let q = virtual_cells.query_selector(qs);

            // We put the negation of the claimed output in the constraint tensor.
            let mut constraints: Tensor<Expression<F>> = Tensor::from(
                (0..OUT).map(|i| -virtual_cells.query_advice(output, Rotation(i as i32))),
            );

            // Now we compute the linear expression,  and add it to constraints
            constraints = constraints.enum_map(|i, mut c| {
                for j in 0..IN {
                    c = c + virtual_cells.query_advice(weights[i], Rotation(j as i32))
                        * virtual_cells.query_advice(input, Rotation(j as i32));
                }
                // add the bias
                c + virtual_cells.query_advice(bias, Rotation(i as i32))
            });

            let constraints = (0..OUT).map(|_| "c").zip(constraints);
            Constraints::with_selector(q, constraints)
        });

        Self {
            weights,
            input,
            output,
            bias,
            q: qs,
            _marker: PhantomData,
        }
    }
}
