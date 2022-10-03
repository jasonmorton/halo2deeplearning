use crate::nn::affine::Affine1dConfig;
use crate::nn::cnvrl::ConvConfig;
use crate::nn::eltwise::{EltwiseConfig, ReLu, ReLu128, ReLu64, Sigmoid};
use crate::nn::LayerConfig;
use crate::tensor::TensorType;
use crate::tensor::{Tensor, ValTensor, VarTensor};
use anyhow::{Context, Result};
use halo2_proofs::{
    arithmetic::FieldExt,
    circuit::{Layouter, Value},
    plonk::{Column, ConstraintSystem, Instance},
};
use std::env;
use std::path::Path;
use tract_onnx;

use tract_onnx::prelude::{Framework, Graph, InferenceFact, Node, OutletId};
use tract_onnx::tract_hir::{infer::Factoid, internal::InferenceOp};

use super::utilities::{ndarray_to_quantized, node_output_shapes};

// Initially, some of these OpKinds will be folded into others (for example, Const nodes that
// contain parameters will be handled at the consuming node.
// Eventually, though, we probably want to keep them and treat them directly (layouting and configuring
// at each type of node)
#[derive(Clone, Debug, Copy)]
pub enum OpKind {
    Affine,
    Convolution,
    ReLU,
    ReLU64,
    ReLU128,
    Sigmoid,
    Const,
    Input,
    Unknown,
}

#[derive(Clone, Debug)]
pub enum OnnxNodeConfig<F: FieldExt + TensorType> {
    Affine(Affine1dConfig<F>),
    Conv(ConvConfig<F>),
    ReLU(EltwiseConfig<F, ReLu<F>>),
    ReLU64(EltwiseConfig<F, ReLu64<F>>),
    ReLU128(EltwiseConfig<F, ReLu128<F>>),
    Sigmoid(EltwiseConfig<F, Sigmoid<F, 128, 128>>),
    Const,
    Input,
    NotConfigured,
}

#[derive(Clone)]
pub struct OnnxModelConfig<F: FieldExt + TensorType> {
    configs: Vec<OnnxNodeConfig<F>>,
    pub public_output: Column<Instance>,
}

/// Fields:
/// node is the raw Tract Node data structure.
/// opkind: OpKind is our op enum.
/// output_max is an inferred maximum value that can appear in the output tensor given previous quantization choices.
/// in_scale and out_scale track the denominator in the fixed point representation. Tensors of differing scales should not be combined.
/// input_shapes and output_shapes  are Option<Vec<Option<Vec<usize>>>>.  These are the inferred shapes for input and output tensors. The first coordinate is the Onnx "slot" and the second is the tensor.  The input_shape includes all the parameters, not just the activations that will flow into the node.
/// None indicates unknown, so input_shapes = Some(vec![None, Some(vec![3,4])]) indicates that we
/// know something, there are two slots, and the first tensor has unknown shape, while the second has shape [3,4].
/// in_dims and out_dims are the shape of the activations only which enter and leave the node.
#[derive(Clone, Debug)]
pub struct OnnxNode {
    node: Node<InferenceFact, Box<dyn InferenceOp>>,
    pub opkind: OpKind,
    output_max: f32,
    in_scale: i32,
    out_scale: i32,
    constant_value: Option<Tensor<i32>>, // float value * 2^qscale if applicable.
    input_shapes: Option<Vec<Option<Vec<usize>>>>,
    output_shapes: Option<Vec<Option<Vec<usize>>>>,
    // Usually there is a simple in and out shape of the node as an operator.  For example, an Affine node has three input_shapes (one for the input, weight, and bias),
    // but in_dim is [in], out_dim is [out]
    in_dims: Option<Vec<usize>>,
    out_dims: Option<Vec<usize>>,
}

impl OnnxNode {
    pub fn new(node: Node<InferenceFact, Box<dyn InferenceOp>>) -> Self {
        let opkind = match node.op().name().as_ref() {
            "Gemm" => OpKind::Affine,
            "Conv" => OpKind::Convolution,
            "Clip" => OpKind::ReLU,
            "Sigmoid" => OpKind::Sigmoid,
            "Const" => OpKind::Const,
            "Source" => OpKind::Input,
            _ => OpKind::Unknown,
        };
        let output_shapes = match node_output_shapes(&node) {
            Ok(s) => Some(s),
            _ => None,
        };

        // Set some default values, then figure out more specific values if possible based on the opkind.
        let mut constant_value = None;
        let mut in_scale = 0i32;
        let mut out_scale = 0i32;
        let in_dims = None;
        let mut out_dims = None;
        let mut output_max = f32::INFINITY;

        match opkind {
            OpKind::Const => {
                let fact = &node.outputs[0].fact;
                let nav = fact
                    .value
                    .concretize()
                    .unwrap()
                    .to_array_view::<f32>()
                    .unwrap()
                    .to_owned();
                out_scale = 7;
                let t = ndarray_to_quantized(nav, 0f32, (i32::pow(2, out_scale as u32) as f32))
                    .unwrap();
                out_dims = Some(t.dims().clone().to_vec());
                output_max = t.iter().map(|x| x.abs()).max().unwrap() as f32;
                constant_value = Some(t);
            }
            OpKind::Input => {
                if let Some([Some(v)]) = output_shapes.as_deref() {
                    out_dims = Some(v.to_vec());
                }
                output_max = 256.0;
                in_scale = 7;
                out_scale = 7;
            }
            _ => {}
        };

        let on = OnnxNode {
            node,
            opkind,
            output_max,
            in_scale,
            out_scale,
            constant_value,
            input_shapes: None,
            output_shapes,
            in_dims,
            out_dims,
        };
        //        println!("New{:?}", on);
        on
    }

    pub fn output_shapes(&self) -> Result<Vec<Option<Vec<usize>>>> {
        let mut shapes = Vec::new();
        let outputs = self.node.outputs.to_vec();
        for output in outputs {
            let mv = output
                .fact
                .shape
                .clone()
                .as_concrete_finite()?
                .map(|x| x.to_vec());
            shapes.push(mv)
        }
        Ok(shapes)
    }

    pub fn name(&self) -> String {
        self.node.name.clone().into()
    }

    pub fn output_ndarray_by_slot(
        &self,
        slot: usize,
    ) -> ndarray::ArrayBase<ndarray::OwnedRepr<f32>, ndarray::Dim<ndarray::IxDynImpl>> {
        let fact = &self.node.outputs[slot].fact;
        let nav = fact
            .value
            .concretize()
            .unwrap()
            .to_array_view::<f32>()
            .unwrap()
            .to_owned();
        nav
    }

    pub fn output_tensor_by_slot(&self, slot: usize, shift: f32, scale: f32) -> Tensor<i32> {
        let arr = self.output_ndarray_by_slot(slot);
        ndarray_to_quantized(arr, shift, scale).unwrap()
    }
}

#[derive(Clone, Debug)]
pub struct OnnxModel {
    pub model: Graph<InferenceFact, Box<dyn InferenceOp>>, // The raw Tract data structure
    pub onnx_nodes: Vec<OnnxNode>, // Wrapped nodes with additional methods and data (e.g. inferred shape, quantization)
    pub bits: usize,
    pub last_shape: Vec<usize>,
}

impl OnnxModel {
    pub fn new(path: impl AsRef<Path>) -> Self {
        let model = tract_onnx::onnx().model_for_path(path).unwrap();

        let onnx_nodes: Vec<OnnxNode> = model
            .nodes()
            .iter()
            .map(|n| OnnxNode::new(n.clone()))
            .collect();
        let mut om = OnnxModel {
            model,
            onnx_nodes,
            bits: 15,
            last_shape: Vec::from([0]),
        };
        om.forward_shape_and_quantize_pass().unwrap();
        om
    }
    pub fn from_arg() -> Self {
        let args: Vec<String> = env::args().collect();
        let filename: String = args[1].clone();
        OnnxModel::new(filename)
    }

    pub fn configure<F: FieldExt + TensorType>(
        &mut self,
        meta: &mut ConstraintSystem<F>,
        advices: VarTensor,
        fixeds: VarTensor,
    ) -> Result<OnnxModelConfig<F>> {
        // Note that the order of the nodes, and the eval_order, is not stable between model loads
        let order = self.eval_order()?;
        let mut configs: Vec<OnnxNodeConfig<F>> = vec![OnnxNodeConfig::NotConfigured; order.len()];
        for node_idx in order {
            configs[node_idx] =
                self.configure_node(node_idx, meta, advices.clone(), fixeds.clone())?;
        }

        println!("Configured: {:?}", configs);
        let public_output: Column<Instance> = meta.instance_column();
        meta.enable_equality(public_output);

        Ok(OnnxModelConfig {
            configs,
            public_output,
        })
    }

    /// Infer the params, input, and output, and configure against the provided meta and Advice and Fixed columns.
    /// Note that we require the context of the Graph to complete this task.
    fn configure_node<F: FieldExt + TensorType>(
        &mut self,
        node_idx: usize,
        meta: &mut ConstraintSystem<F>,
        advices: VarTensor,
        fixeds: VarTensor, // Should use fixeds, but currently buggy
    ) -> Result<OnnxNodeConfig<F>> {
        let node = &self.onnx_nodes[node_idx];
        //        println!("Configure Node {}, a {:?}", node_idx, node.opkind());

        // Figure out, find, and load the params
        match node.opkind {
            OpKind::Affine => {
                // The parameters are assumed to be fixed kernel and bias. This node should have three inputs in total:
                // two inputs which are Const(..) that have the f32s, and one variable input which are the activations.
                // The first input is the activations, second is the weight matrix, and the third the bias.
                // Consider using shape information only here, rather than loading the param tensor (although loading
                // the tensor guarantees that assign will work if there are errors or ambiguities in the shape
                // data).
                let input_outlets = &node.node.inputs;
                let (_input_node_ix, _input_node_slot) =
                    (input_outlets[0].node, input_outlets[0].slot);

                let (weight_node_ix, _weight_node_slot) =
                    (input_outlets[1].node, input_outlets[1].slot);
                let (_bias_node_ix, _bias_node_slot) =
                    (input_outlets[2].node, input_outlets[2].slot);
                let weight_node = &self.onnx_nodes[weight_node_ix];
                let weight_value = weight_node
                    .constant_value
                    .clone()
                    .context("Tensor<i32> should already be loaded")?;
                // let weight_value =
                //     weight_node.output_tensor_by_slot(weight_node_slot, 0f32, 128f32);

                let in_dim = weight_value.dims()[1];
                let out_dim = weight_value.dims()[0];
                // If we don't want to resue the columns:
                // let mut weight: Tensor<Column<Fixed>> =
                //     (0..in_dim * out_dim).map(|_| meta.fixed_column()).into();
                // weight.reshape(&weight_value.dims());
                // let mut bias: Tensor<Column<Fixed>> =
                //     (0..out_dim).map(|_| meta.fixed_column()).into();
                // bias.reshape?
                let weight_fixeds = advices.get_slice(&[0..out_dim], &[out_dim, in_dim]); //&[0..out_dim], &[out_dim, in_dim]
                let bias_fixeds = advices.get_slice(&[out_dim + 1..out_dim + 2], &[out_dim]);
                let input = advices.get_slice(&[out_dim + 2..out_dim + 3], &[in_dim]);
                let output = advices.get_slice(&[out_dim + 3..out_dim + 4], &[out_dim]);
                let conf = Affine1dConfig::configure(
                    meta,
                    &[weight_fixeds, bias_fixeds, input, output],
                    None,
                );
                self.last_shape = Vec::from([out_dim]);
                Ok(OnnxNodeConfig::Affine(conf))
            }
            OpKind::ReLU => {
                let length = self.last_shape.clone().into_iter().product();

                let conf: EltwiseConfig<F, ReLu<F>> = EltwiseConfig::configure(
                    meta,
                    &[advices.get_slice(&[0..length], &[length])],
                    Some(&[self.bits]),
                );
                Ok(OnnxNodeConfig::ReLU(conf))
            }
            OpKind::ReLU64 => {
                let length = self.last_shape.clone().into_iter().product();

                let conf: EltwiseConfig<F, ReLu64<F>> = EltwiseConfig::configure(
                    meta,
                    &[advices.get_slice(&[0..length], &[length])],
                    Some(&[self.bits]),
                );
                Ok(OnnxNodeConfig::ReLU64(conf))
            }
            OpKind::ReLU128 => {
                let length = self.last_shape.clone().into_iter().product();

                let conf: EltwiseConfig<F, ReLu128<F>> = EltwiseConfig::configure(
                    meta,
                    &[advices.get_slice(&[0..length], &[length])],
                    Some(&[self.bits]),
                );
                Ok(OnnxNodeConfig::ReLU128(conf))
            }

            OpKind::Sigmoid => {
                // Here,   node.output_shapes().unwrap()[0].as_ref().unwrap() == vec![1,LEN]
                let length = node.output_shapes().unwrap()[0].as_ref().unwrap()[1];
                let conf: EltwiseConfig<F, Sigmoid<F, 128, 128>> = EltwiseConfig::configure(
                    meta,
                    &[advices.get_slice(&[0..length], &[length])],
                    Some(&[self.bits]),
                );
                Ok(OnnxNodeConfig::Sigmoid(conf))
            }
            OpKind::Const => {
                // Typically parameters for one or more layers.
                // Currently this is handled in the consuming node(s), but will be moved here.
                Ok(OnnxNodeConfig::Const)
            }
            OpKind::Input => {
                // This is the input to the model (e.g. the image).
                // Currently this is handled in the consuming node(s), but will be moved here.
                Ok(OnnxNodeConfig::Input)
            }

            _ => {
                unimplemented!()
            }
        }
    }

    pub fn layout<F: FieldExt + TensorType>(
        &self,
        config: OnnxModelConfig<F>,
        layouter: &mut impl Layouter<F>,
        input: ValTensor<F>,
    ) -> Result<ValTensor<F>> {
        let order = self.eval_order()?;
        let mut x = input;
        for node_idx in order {
            println!("Applying {:?}", self.onnx_nodes[node_idx]);
            x = match self.layout_node(
                node_idx,
                layouter,
                x.clone(),
                config.configs[node_idx].clone(),
            )? {
                Some(vt) => {
                    //                    println!("Applying {:?}", self.onnx_nodes[node_idx]);
                    println!("Node {} out: {}", node_idx, vt.show());

                    vt
                }
                None => {
                    println!("Node {} out: {:?}", node_idx, x.show());
                    x
                } // Some nodes don't produce tensor output, we skip these
            }
        }
        Ok(x)
    }

    // Takes an input ValTensor; alternatively we could recursively layout all the predecessor tensors
    // (which may be more correct for some graphs).
    // Does not take parameters, instead looking them up in the network.
    // At the Source level, the input will be fed by the prover.
    fn layout_node<F: FieldExt + TensorType>(
        &self,
        node_idx: usize,
        layouter: &mut impl Layouter<F>,
        input: ValTensor<F>,
        config: OnnxNodeConfig<F>,
    ) -> Result<Option<ValTensor<F>>> {
        let node = &self.onnx_nodes[node_idx];
        let input_outlets = &node.node.inputs;

        //        println!("Layout Node {}, {:?}", node_idx, node.opkind());
        //        let nice: Tensor<i32> = input.into();
        //        println!("Node {} input tensor {:?}", node_idx, &input.clone());

        // The node kind and the config should be the same.
        Ok(match (node.opkind, config.clone()) {
            (OpKind::Affine, OnnxNodeConfig::Affine(ac)) => {
                // This node should have three inputs in total:
                // two inputs which are Const(..) that have the f32s,
                // and one variable input which are the activations.
                // The first input is the activations, second is the weight matrix, and the third the bias.
                // Determine the input, and recursively assign the input node.

                let (_input_node_ix, _input_node_slot) =
                    (input_outlets[0].node, input_outlets[0].slot);
                // Properly we should check that weight and bias are Const ops
                let (weight_node_ix, _weight_node_slot) =
                    (input_outlets[1].node, input_outlets[1].slot);
                let (bias_node_ix, _bias_node_slot) =
                    (input_outlets[2].node, input_outlets[2].slot);
                let weight_node = &self.onnx_nodes[weight_node_ix];
                let weight_value = weight_node
                    .constant_value
                    .clone()
                    .context("Tensor<i32> should already be loaded")?;

                let weight_vt =
                    ValTensor::from(<Tensor<i32> as Into<Tensor<Value<F>>>>::into(weight_value));

                let bias_node = &self.onnx_nodes[bias_node_ix];
                let bias_value = bias_node
                    .constant_value
                    .clone()
                    .context("Tensor<i32> should already be loaded")?;
                let bias_vt =
                    ValTensor::from(<Tensor<i32> as Into<Tensor<Value<F>>>>::into(bias_value));
                let out = ac.layout(layouter, &[weight_vt, bias_vt, input]);
                Some(out)
            }
            (OpKind::Convolution, OnnxNodeConfig::Conv(cc)) => {
                todo!()
            }
            (OpKind::ReLU, OnnxNodeConfig::ReLU(rc)) => {
                // For activations and elementwise operations, the dimensions are sometimes only in one or the other of input and output.
                //                let length = node.output_shapes().unwrap()[0].as_ref().unwrap()[1]; //  shape is vec![1,LEN]
                Some(rc.layout(layouter, &[input]))
            }
            (OpKind::ReLU64, OnnxNodeConfig::ReLU64(rc)) => {
                // For activations and elementwise operations, the dimensions are sometimes only in one or the other of input and output.
                //                let length = node.output_shapes().unwrap()[0].as_ref().unwrap()[1]; //  shape is vec![1,LEN]
                Some(rc.layout(layouter, &[input]))
            }
            (OpKind::ReLU128, OnnxNodeConfig::ReLU128(rc)) => {
                // For activations and elementwise operations, the dimensions are sometimes only in one or the other of input and output.
                //                let length = node.output_shapes().unwrap()[0].as_ref().unwrap()[1]; //  shape is vec![1,LEN]
                Some(rc.layout(layouter, &[input]))
            }
            (OpKind::Sigmoid, OnnxNodeConfig::Sigmoid(sc)) => Some(sc.layout(layouter, &[input])),

            (OpKind::Input, OnnxNodeConfig::Input) => None,
            (OpKind::Const, OnnxNodeConfig::Const) => None,
            _ => {
                panic!(
                    "Node Op and Config mismatch, or unknown Op. {:?} vs {:?}",
                    node.opkind, config
                )
            }
        })
    }

    /// Make a forward pass over the graph to determine tensor shapes and quantization strategy
    /// Mutates the nodes.
    pub fn forward_shape_and_quantize_pass(&mut self) -> Result<()> {
        let order = self.eval_order()?;
        for node_idx in order {
            // mutate a copy of the node, referring to other nodes in the vec, then swap modified node in at the end
            let mut this_node = self.onnx_nodes[node_idx].clone();
            match this_node.opkind {
                OpKind::Affine => {
                    let input_outlets = &this_node.node.inputs;
                    let (input_node_ix, _input_node_slot) =
                        (input_outlets[0].node, input_outlets[0].slot);
                    let (weight_node_ix, _weight_node_slot) =
                        (input_outlets[1].node, input_outlets[1].slot);
                    let (bias_node_ix, _bias_node_slot) =
                        (input_outlets[2].node, input_outlets[2].slot);
                    let input_node = &self.onnx_nodes[input_node_ix];
                    let weight_node = &self.onnx_nodes[weight_node_ix];
                    let bias_node = &self.onnx_nodes[bias_node_ix];
                    let in_dim = weight_node.out_dims.as_ref().unwrap()[1];
                    let out_dim = weight_node.out_dims.as_ref().unwrap()[0];
                    this_node.in_dims = Some(vec![in_dim]);
                    this_node.out_dims = Some(vec![out_dim]);

                    this_node.output_max =
                        input_node.output_max * weight_node.output_max * (in_dim as f32);
                    assert_eq!(input_node.out_scale, weight_node.out_scale);
                    assert_eq!(input_node.out_scale, bias_node.out_scale);
                    this_node.in_scale = input_node.out_scale;
                    this_node.out_scale = weight_node.out_scale + input_node.out_scale;
                }
                OpKind::ReLU => {
                    let input_outlets = &this_node.node.inputs;
                    let (input_node_ix, _input_node_slot) =
                        (input_outlets[0].node, input_outlets[0].slot);
                    let input_node = &self.onnx_nodes[input_node_ix];
                    this_node.in_dims = input_node.out_dims.clone();
                    this_node.out_dims = input_node.out_dims.clone();

                    if this_node.input_shapes == None {
                        this_node.input_shapes = Some(vec![this_node.in_dims.clone()]);
                    }
                    if this_node.output_shapes == None {
                        this_node.output_shapes = Some(vec![this_node.out_dims.clone()]);
                    }
                    this_node.output_max = input_node.output_max;
                    this_node.in_scale = input_node.out_scale;

                    // We can also consider adjusting the scale of all inputs and the output in a more custom way.
                    if this_node.in_scale == 14 {
                        this_node.opkind = OpKind::ReLU128;
                        this_node.output_max = input_node.output_max / 128f32;
                        this_node.out_scale = this_node.in_scale - 7;
                    }

                    // if this_node.output_max > 65536f32 {
                    //     this_node.opkind = OpKind::ReLU128;
                    //     this_node.output_max = input_node.output_max / 128f32;
                    //     this_node.out_scale = input_node.out_scale - 7;
                    // } else if this_node.output_max > 16384f32 {
                    //       this_node.opkind = OpKind::ReLU64;
                    //       this_node.output_max = input_node.output_max / 64f32;
                    //       this_node.out_scale = input_node.out_scale - 6;
                    // }
                }
                _ => {}
            };
            //            println!("mod{:?}", this_node);
            self.onnx_nodes[node_idx] = this_node;
        }

        Ok(())
    }

    // Make a recursive backward pass to shape and quantize?

    /// Get a linear extension of the model (an evaluation order), for example to feed to circuit construction.
    /// Note that this order is not stable over multiple reloads of the model.  For example, it will freely
    /// interchange the order of evaluation of fixed parameters.   For example weight could have id 1 on one load,
    /// and bias id 2, and vice versa on the next load of the same file. The ids are also not stable.
    pub fn eval_order(&self) -> Result<Vec<usize>> {
        self.model.eval_order()
    }

    /// Note that this order is not stable.
    pub fn nodes(&self) -> Vec<Node<InferenceFact, Box<dyn InferenceOp>>> {
        self.model.nodes().clone().to_vec()
    }

    pub fn input_outlets(&self) -> Result<Vec<OutletId>> {
        Ok(self.model.input_outlets()?.to_vec())
    }

    pub fn output_outlets(&self) -> Result<Vec<OutletId>> {
        Ok(self.model.output_outlets()?.to_vec())
    }

    // In general we need to determine the input shapes by looking up the input tensors in the predeccesor nodes
    pub fn input_shapes_by_node_idx(&self, node_idx: usize) -> Result<Vec<Option<Vec<usize>>>> {
        let mut shapes = Vec::new();

        for OutletId { node, slot } in &self.model.nodes()[node_idx].inputs {
            //            let prec = OnnxNode::new(self.model.nodes()[*node].clone());
            let prec = &self.onnx_nodes[*node];
            let shapevec = prec.output_shapes()?;
            shapes.push(shapevec[*slot].clone());
        }
        Ok(shapes)
    }

    pub fn max_fixeds_width(&self) -> Result<usize> {
        self.max_advices_width() //todo, improve this computation
    }

    pub fn max_advices_width(&self) -> Result<usize> {
        let mut max: usize = 1;
        for node in &self.model.nodes {
            for shape in node_output_shapes(&node)? {
                match shape {
                    None => {}
                    Some(vs) => {
                        for v in vs {
                            if v > max {
                                max = v
                            }
                        }
                    }
                }
            }
        }
        Ok(max + 5)
    }

    pub fn get_node_output_shape_by_name_and_rank(
        &self,
        name: impl AsRef<str>,
        rank: usize,
    ) -> Vec<usize> {
        let node = self.model.node_by_name(name).unwrap();
        let fact = &node.outputs[rank].fact;
        let shape = fact.shape.clone().as_concrete_finite().unwrap().unwrap();
        shape.to_vec()
    }

    pub fn get_ndarray_by_node_name(
        &self,
        name: impl AsRef<str>,
    ) -> ndarray::ArrayBase<ndarray::OwnedRepr<f32>, ndarray::Dim<ndarray::IxDynImpl>> {
        let node = self.model.node_by_name(name).unwrap();
        let fact = &node.outputs[0].fact;

        let nav = fact
            .value
            .concretize()
            .unwrap()
            .to_array_view::<f32>()
            .unwrap()
            .to_owned();
        nav
    }

    pub fn get_tensor_by_node_name(
        &self,
        name: impl AsRef<str>,
        shift: f32,
        scale: f32,
    ) -> Tensor<i32> {
        let arr = self.get_ndarray_by_node_name(name);
        ndarray_to_quantized(arr, shift, scale).unwrap()
    }
}
