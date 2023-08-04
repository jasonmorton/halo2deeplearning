use super::utilities::node_output_shapes;
use super::Visibility;
use crate::circuit::Op;
use crate::graph::new_op_from_onnx;
use halo2curves::bn256::Fr as Fp;
use log::trace;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use tabled::Tabled;
use tract_onnx;
use tract_onnx::prelude::Node as OnnxNode;
use tract_onnx::prelude::TypedFact;
use tract_onnx::prelude::TypedOp;

fn display_vector<T: fmt::Debug>(v: &Vec<T>) -> String {
    if !v.is_empty() {
        format!("{:?}", v)
    } else {
        String::new()
    }
}

#[allow(clippy::borrowed_box)]
fn display_opkind(v: &Box<dyn Op<Fp>>) -> String {
    v.as_string()
}

/// A single operation in a [crate::graph::Model].
#[derive(Clone, Debug, Tabled, Serialize, Deserialize)]
pub struct Node {
    /// [Op] i.e what operation this node represents.
    #[tabled(display_with = "display_opkind")]
    #[serde(with = "serde_traitobject")]
    pub opkind: Box<dyn Op<Fp>>,
    /// The denominator in the fixed point representation for the node's output. Tensors of differing scales should not be combined.
    pub out_scale: u32,
    // Usually there is a simple in and out shape of the node as an operator.  For example, an Affine node has three input_shapes (one for the input, weight, and bias),
    // but in_dim is [in], out_dim is [out]
    #[tabled(display_with = "display_vector")]
    /// The indices of the node's inputs.
    pub inputs: Vec<usize>,
    #[tabled(display_with = "display_vector")]
    /// Dimensions of output.
    pub out_dims: Vec<usize>,
    /// The node's unique identifier.
    pub idx: usize,
}

impl PartialEq for Node {
    fn eq(&self, other: &Node) -> bool {
        (self.out_scale == other.out_scale)
            && (self.inputs == other.inputs)
            && (self.out_dims == other.out_dims)
            && (self.idx == other.idx)
            && (self.opkind.as_string() == other.opkind.as_string())
    }
}

impl Node {
    /// Converts a tract [OnnxNode] into an ezkl [Node].
    /// # Arguments:
    /// * `node` - [OnnxNode]
    /// * `other_nodes` - [BTreeMap] of other previously initialized [Node]s in the computational graph.
    /// * `public_params` - flag if parameters of model are public
    /// * `idx` - The node's unique identifier.
    pub fn new(
        node: OnnxNode<TypedFact, Box<dyn TypedOp>>,
        other_nodes: &mut BTreeMap<usize, super::NodeType>,
        scale: u32,
        param_visibility: Visibility,
        idx: usize,
    ) -> Result<Self, Box<dyn Error>> {
        trace!("Create {:?}", node);
        trace!("Create op {:?}", node.op);

        // load the node inputs
        let mut inputs = vec![];

        // we can only take the inputs as mutable once -- so we need to collect them first
        let mut input_ids = node.inputs.iter().map(|i| i.node).collect::<Vec<_>>();

        other_nodes.iter_mut().for_each(|(i, v)| {
            if input_ids.contains(i) {
                inputs.push(v);
            }
        });

        inputs.sort_by(|a, b| {
            let a_idx = input_ids.iter().position(|&x| x == a.idx()).unwrap();
            let b_idx = input_ids.iter().position(|&x| x == b.idx()).unwrap();
            a_idx.cmp(&b_idx)
        });

        let mut opkind = new_op_from_onnx(idx, scale, param_visibility, node.clone(), &mut inputs)?; // parses the op name

        // we can only take the inputs as mutable once -- so we need to collect them first
        let remaining_inputs = inputs.iter().map(|i| i.idx()).collect::<Vec<_>>();
        input_ids.retain(|&x| remaining_inputs.contains(&x));

        // rescale the inputs if necessary to get consistent fixed points
        let in_scales: Vec<u32> = input_ids
            .iter()
            .map(|n| {
                let idx = inputs.iter().position(|x| *n == x.idx()).unwrap();
                inputs[idx].out_scales()[0]
            })
            .collect();
        opkind = opkind.rescale(in_scales.clone(), scale);
        let out_scale = match in_scales.len() {
            0 => scale,
            _ => opkind.out_scale(in_scales, scale),
        };

        // get the output shape
        let out_dims = {
            let output_shapes = match node_output_shapes(&node) {
                Ok(s) => Some(s),
                _ => None,
            };

            if let Some([Some(v)]) = output_shapes.as_deref() {
                v.to_vec()
            } else {
                panic!("Could not get output shape for node {:?}", node);
            }
        };

        Ok(Node {
            idx,
            opkind,
            inputs: input_ids,
            out_dims,
            out_scale,
        })
    }
}
