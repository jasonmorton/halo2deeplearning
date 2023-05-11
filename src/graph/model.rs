use super::node::*;
use super::vars::*;
use super::GraphError;
use super::ModelParams;
use crate::{
    circuit::{lookup::LookupOp, ops::poly::PolyOp, BaseConfig as PolyConfig, CheckMode, Op},
    commands::{Cli, Commands, RunArgs},
    graph::scale_to_multiplier,
    tensor::{Tensor, TensorType, ValTensor},
};

use halo2curves::ff::PrimeField;
use log::warn;
use serde::Deserialize;
use serde::Serialize;
use tract_onnx::prelude::{
    DatumExt, Graph, InferenceFact, InferenceModelExt, SymbolValues, TypedFact, TypedOp,
};

// use tract_onnx::tract_hir::internal::GenericFactoid;
//use clap::Parser;
use core::panic;
use halo2_proofs::{
    circuit::{Layouter, Value},
    plonk::ConstraintSystem,
};
use itertools::Itertools;
use log::error;
use log::{debug, info, trace};
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::error::Error;
use tabled::Table;
use tract_onnx;
use tract_onnx::prelude::Framework;
/// Mode we're using the model in.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub enum Mode {
    /// Initialize the model and display the operations table / graph
    #[default]
    Table,
    /// Initialize the model and generate a mock proof
    Mock,
    /// Initialize the model and generate a proof
    Prove,
    /// Initialize the model, generate a proof, and verify
    FullProve,
    /// Initialize the model and verify an already generated proof
    Verify,
}

/// A circuit configuration for the entirety of a model loaded from an Onnx file.
#[derive(Clone, Debug)]
pub struct ModelConfig<F: PrimeField + TensorType + PartialOrd> {
    /// The base configuration for the circuit
    pub base: PolyConfig<F>,
    /// A wrapper for holding all columns that will be assigned to by the model
    pub vars: ModelVars<F>,
}

/// A struct for loading from an Onnx file and converting a computational graph to a circuit.
#[derive(Clone, Debug, Default)]
pub struct Model<F: PrimeField + TensorType + PartialOrd> {
    /// input indices
    pub inputs: Vec<usize>,
    /// output indices
    pub outputs: Vec<usize>,
    /// Graph of nodes we are loading from Onnx.
    pub nodes: NodeGraph<F>, // Wrapped nodes with additional methods and data (e.g. inferred shape, quantization)
    /// The [RunArgs] being used
    pub run_args: RunArgs,
    /// The [Mode] we're using the model in.
    pub mode: Mode,
    /// Defines which inputs to the model are public and private (params, inputs, outputs) using [VarVisibility].
    pub visibility: VarVisibility,
}

/// Enables model as subnode of other models
#[derive(Clone, Debug)]
pub enum NodeTypes<F: PrimeField + TensorType + PartialOrd> {
    /// A node in the model
    Node(Node<F>),
    /// A submodel
    Model(Model<F>),
}

impl<F: PrimeField + TensorType + PartialOrd> Model<F> {
    /// Creates an `Model` from a specified path to an Onnx file.
    /// # Arguments
    /// * `reader` - A reader for an Onnx file.
    /// * `run_args` - [RunArgs]
    /// * `mode` - The [Mode] we're using the model in.
    /// * `visibility` - Which inputs to the model are public and private (params, inputs, outputs) using [VarVisibility].
    pub fn new(
        reader: &mut dyn std::io::Read,
        run_args: RunArgs,
        mode: Mode,
        visibility: VarVisibility,
    ) -> Result<Self, Box<dyn Error>> {
        let (model, nodes) = Self::load_onnx_model(reader, run_args.scale, run_args.public_params)?;

        let om = Model {
            inputs: model.inputs.iter().map(|o| o.node).collect(),
            outputs: model.outputs.iter().map(|o| o.node).collect(),
            run_args,
            nodes,
            mode,
            visibility,
        };

        Ok(om)
    }

    /// Generate model parameters for the circuit
    pub fn gen_params(&self, check_mode: CheckMode) -> Result<ModelParams, Box<dyn Error>> {
        let instance_shapes = self.instance_shapes();
        // this is the total number of variables we will need to allocate
        // for the circuit
        let num_constraints = if let Some(num_constraints) = self.run_args.allocated_constraints {
            num_constraints
        } else {
            self.dummy_layout(&self.input_shapes()).unwrap()
        };

        // extract the requisite lookup ops from the model
        let mut lookup_ops: Vec<LookupOp> = self
            .nodes
            .iter()
            .map(|(_, n)| n.opkind.required_lookups())
            .flatten()
            .collect();

        let set: HashSet<_> = lookup_ops.drain(..).collect(); // dedup
        lookup_ops.extend(set.into_iter().sorted());

        Ok(ModelParams {
            run_args: self.run_args.clone(),
            visibility: self.visibility.clone(),
            instance_shapes,
            num_constraints,
            required_lookups: lookup_ops,
            check_mode,
        })
    }

    /// Runs a forward pass on sample data !
    /// # Arguments
    /// * `reader` - A reader for an Onnx file.
    /// * `model_inputs` - A vector of [Tensor]s to use as inputs to the model.
    /// * `run_args` - [RunArgs]
    pub fn forward(
        &self,
        model_inputs: &[Tensor<i128>],
    ) -> Result<Vec<Tensor<f32>>, Box<dyn Error>> {
        let mut results: BTreeMap<&usize, Tensor<i128>> = BTreeMap::new();
        let mut max_lookup_inputs = 0;
        let mut input_idx = 0;
        for (i, n) in self.nodes.iter() {
            let mut inputs = vec![];
            if n.opkind.is_input() {
                let mut t = model_inputs[input_idx].clone();
                input_idx += 1;
                t.reshape(&n.out_dims);
                inputs.push(t);
            } else {
                debug!("executing {}: {}", i, n.opkind.as_str());
                trace!("dims: {:?}", n.out_dims);
                for i in n.inputs.iter() {
                    match results.get(&i) {
                        Some(value) => inputs.push(value.clone()),
                        None => return Err(Box::new(GraphError::MissingNode(*i))),
                    }
                }
            };

            if n.opkind.required_lookups().len() > 0 {
                let mut max = 0;
                for i in &inputs {
                    max = max.max(i.iter().map(|x| x.abs()).max().unwrap());
                }
                max_lookup_inputs = max_lookup_inputs.max(max);
            }

            let res = Op::<F>::f(&*n.opkind, &inputs)?;
            results.insert(i, res);
        }

        let output_nodes = self.outputs.iter();
        debug!(
            "model outputs are nodes: {:?}",
            output_nodes.clone().map(|o| o).collect_vec()
        );
        let outputs = output_nodes
            .map(|o| {
                let n = self.nodes.get(&o).unwrap();
                let scale = scale_to_multiplier(n.out_scale);
                results.get(&o).unwrap().clone().map(|x| (x as f32) / scale)
            })
            .collect_vec();

        let max_range = 2i128.pow(self.run_args.bits as u32 - 1);
        if max_lookup_inputs >= max_range {
            let recommended_bits = (max_lookup_inputs as f64).log2().ceil() as u32 + 1;
            let recommended_scale = 1.0
                + (max_lookup_inputs as f64 / max_range as f64).log2().ceil()
                - self.run_args.scale as f64;
            warn!("At the selected lookup bits and fixed point scale, the largest input to a lookup table is too large to be represented (max: {}, bits: {}, scale: {}).",  max_lookup_inputs, self.run_args.bits, self.run_args.scale);
            if recommended_scale > 0.0 {
                warn!("Either increase the lookup bits to [{}] or decrease the scale to [{}] (or both).", recommended_bits, recommended_scale);
                warn!("Remember to increase the circuit logrows if you increase the bits.");
                warn!("Remember to re-run the forward pass with the new values.");
            } else if recommended_bits <= 27 {
                warn!("Increase the lookup bits to [{}]. The current scale cannot be decreased enough to fit the largest lookup input. ", recommended_bits);
                warn!("Remember to increase the circuit logrows if you increase the bits.");
                warn!("Remember to re-run the forward pass with the new values.");
            } else {
                let max_range = 2i128.pow(27_u32 - 1);
                let recommended_scale = self.run_args.scale as f64
                    - (max_lookup_inputs as f64 / max_range as f64).log2().ceil();
                if recommended_scale > 0.0 {
                    warn!(
                        "Increase the bits to [27] and the scale to [{}]",
                        recommended_scale
                    );
                    warn!("Remember to increase the circuit logrows if you increase the bits.");
                    warn!("Remember to re-run the forward pass with the new values.");
                } else {
                    warn!("No possible value of bits or scale can accomodate this value.")
                }
            }
        }

        Ok(outputs)
    }

    /// Loads an Onnx model from a specified path.
    /// # Arguments
    /// * `reader` - A reader for an Onnx file.
    /// * `scale` - The scale to use for quantization.
    /// * `public_params` - Whether to make the params public.
    fn load_onnx_model(
        reader: &mut dyn std::io::Read,
        scale: u32,
        public_params: bool,
    ) -> Result<(Graph<TypedFact, Box<dyn TypedOp>>, BTreeMap<usize, Node<F>>), Box<dyn Error>>
    {
        let mut model = tract_onnx::onnx().model_for_read(reader).map_err(|e| {
            error!("Error loading model: {}", e);
            GraphError::ModelLoad
        })?;

        for (i, id) in model.clone().inputs.iter().enumerate() {
            let input = model.node(id.node);

            let mut dims = vec![];
            let extracted_dims: Vec<usize> = input.outputs[0]
                .fact
                .shape
                .dims()
                .filter_map(|x| tract_onnx::tract_hir::internal::Factoid::concretize(x))
                .map(|x| match x.to_i64() {
                    Ok(x) => x as usize,
                    Err(_e) => {
                        if x.to_string() == "batch_size" {
                            1
                        } else {
                            panic!("Unknown dimension {}: {:?}", x.to_string(), x)
                        }
                    }
                })
                .collect();

            dims.extend(extracted_dims);

            model.set_input_fact(i, f32::fact(dims).into())?;
        }

        for (i, _) in model.clone().outputs.iter().enumerate() {
            model.set_output_fact(i, InferenceFact::default()).unwrap();
        }
        // Note: do not optimize the model, as the layout will depend on underlying hardware
        let model = model.into_typed()?.into_decluttered()?;
        let batch_size = model.symbol_table.sym("batch_size");
        let seq_len = model.symbol_table.sym("sequence_length");
        let model = model
            .concretize_dims(&SymbolValues::default().with(&batch_size, 1))?
            .concretize_dims(&SymbolValues::default().with(&seq_len, 1))?;

        let mut nodes = BTreeMap::<usize, Node<F>>::new();
        for (i, n) in model.nodes.iter().enumerate() {
            let n = Node::<F>::new(n.clone(), &mut nodes, scale, public_params, i)?;
            nodes.insert(i, n);
        }

        nodes = nodes
            .iter()
            .filter(|(_, node)| {
                node.opkind
                    .as_any()
                    .downcast_ref::<crate::circuit::ops::Constant<F>>()
                    .is_none()
            })
            .map(|(idx, node)| (*idx, node.clone()))
            .collect();

        debug!("\n {}", model);

        debug!("\n {}", Table::new(nodes.iter()).to_string());

        Ok((model, nodes))
    }

    /// Creates a `Model` from parsed CLI arguments
    /// # Arguments
    /// * `cli` - A [Cli] struct holding parsed CLI arguments.
    pub fn from_ezkl_conf(cli: Cli) -> Result<Self, Box<dyn Error>> {
        match cli.command {
            Commands::Table { model, args, .. } | Commands::Mock { model, args, .. } => {
                let visibility = VarVisibility::from_args(args.clone())?;
                Model::new(
                    &mut std::fs::File::open(model)?,
                    args,
                    Mode::Mock,
                    visibility,
                )
            }
            Commands::Setup { model, args, .. } => {
                let visibility = VarVisibility::from_args(args.clone())?;
                Model::new(
                    &mut std::fs::File::open(model)?,
                    args,
                    Mode::Prove,
                    visibility,
                )
            }
            #[cfg(feature = "render")]
            Commands::RenderCircuit { model, args, .. } => {
                let visibility = VarVisibility::from_args(args.clone())?;
                Model::new(
                    &mut std::fs::File::open(model)?,
                    args,
                    Mode::Table,
                    visibility,
                )
            }
            _ => panic!(),
        }
    }

    /// Creates a `Model` from parsed model params
    /// # Arguments
    /// * `params` - A [ModelParams] struct holding parsed CLI arguments.
    pub fn from_model_params(
        params: &ModelParams,
        model: &std::path::PathBuf,
    ) -> Result<Self, Box<dyn Error>> {
        let visibility = VarVisibility::from_args(params.run_args.clone())?;
        Model::new(
            &mut std::fs::File::open(model)?,
            params.run_args.clone(),
            Mode::Prove,
            visibility,
        )
    }

    /// Creates a `Model` based on CLI arguments
    pub fn from_arg() -> Result<Self, Box<dyn Error>> {
        let conf = Cli::create()?;
        Self::from_ezkl_conf(conf)
    }

    /// Configures a model for the circuit
    /// # Arguments
    /// * `meta` - The constraint system.
    /// * `vars` - The variables for the circuit.
    /// * `run_args` - [RunArgs]
    /// * `required_lookups` - The required lookup operations for the circuit.
    pub fn configure(
        meta: &mut ConstraintSystem<F>,
        vars: &mut ModelVars<F>,
        num_bits: usize,
        tolerance: i32,
        required_lookups: Vec<LookupOp>,
        check_mode: CheckMode,
    ) -> Result<PolyConfig<F>, Box<dyn Error>> {
        info!("configuring model");
        let mut base_gate = PolyConfig::configure(
            meta,
            vars.advices[0..2].try_into()?,
            &vars.advices[2],
            check_mode,
            tolerance,
        );

        for op in required_lookups {
            let input = &vars.advices[0];
            let output = &vars.advices[1];
            base_gate.configure_lookup(meta, input, output, num_bits, &op)?;
        }

        Ok(base_gate)
    }

    /// Assigns values to the regions created when calling `configure`.
    /// # Arguments
    /// * `config` - [ModelConfig] holding all node configs.
    /// * `layouter` - Halo2 Layouter.
    /// * `inputs` - The values to feed into the circuit.
    /// * `vars` - The variables for the circuit.
    pub fn layout(
        &self,
        mut config: ModelConfig<F>,
        layouter: &mut impl Layouter<F>,
        inputs: &[ValTensor<F>],
        vars: &ModelVars<F>,
    ) -> Result<(), Box<dyn Error>> {
        info!("model layout...");
        let mut results = BTreeMap::<usize, ValTensor<F>>::new();
        for (i, input_idx) in self.inputs.iter().enumerate() {
            if self.visibility.input.is_public() {
                results.insert(*input_idx, vars.instances[i].clone());
            } else {
                results.insert(*input_idx, inputs[i].clone());
            }
        }

        config.base.layout_tables(layouter)?;

        layouter.assign_region(
            || "model",
            |mut region| {
                let mut offset: usize = 0;
                for (idx, node) in self.nodes.iter() {
                    let values: Vec<ValTensor<F>> = node
                        .inputs
                        .iter()
                        .map(|i| results.get(i).unwrap().clone())
                        .collect_vec();

                    debug!(
                        "laying out {}: {}, offset:{}",
                        idx,
                        node.opkind.as_str(),
                        offset
                    );
                    trace!("dims: {:?}", node.out_dims);
                    let res = config
                        .base
                        .layout(
                            &mut Some(&mut region),
                            &values,
                            &mut offset,
                            node.opkind.clone_dyn(),
                        )
                        .map_err(|e| {
                            error!("{}", e);
                            halo2_proofs::plonk::Error::Synthesis
                        })?;

                    if let Some(vt) = res {
                        // we get the max as for fused nodes this corresponds to the node output
                        results.insert(*idx, vt);
                        //only use with mock prover
                        if matches!(self.mode, Mode::Mock) {
                            trace!(
                                "------------ output node {:?}: {:?}",
                                idx,
                                results.get(idx).unwrap().show()
                            );
                        }
                    }
                }

                let output_nodes = self.outputs.iter();
                debug!(
                    "model outputs are nodes: {:?}",
                    output_nodes.clone().collect_vec()
                );
                let mut outputs = output_nodes
                    .map(|o| results.get(o).unwrap().clone())
                    .collect_vec();

                // pack outputs if need be
                if self.run_args.pack_base > 1 {
                    for i in 0..outputs.len() {
                        debug!("packing outputs...");
                        outputs[i] = config
                            .base
                            .layout(
                                &mut Some(&mut region),
                                &outputs[i..i + 1],
                                &mut offset,
                                Box::new(PolyOp::Pack(
                                    self.run_args.pack_base,
                                    self.run_args.scale,
                                )),
                            )
                            .map_err(|e| {
                                error!("{}", e);
                                halo2_proofs::plonk::Error::Synthesis
                            })?
                            .unwrap();
                        // only use with mock prover
                        if matches!(self.mode, Mode::Mock) {
                            trace!("------------ packed output {:?}", outputs[i].show());
                        }
                    }
                }

                if self.run_args.public_outputs {
                    let _ = outputs
                        .into_iter()
                        .enumerate()
                        .map(|(i, output)| {
                            let mut instance_offset = 0;
                            if self.visibility.input.is_public() {
                                instance_offset += inputs.len();
                            };
                            config.base.layout(
                                &mut Some(&mut region),
                                &[output, vars.instances[instance_offset + i].clone()],
                                &mut offset,
                                Box::new(PolyOp::RangeCheck(self.run_args.tolerance as i32)),
                            )
                        })
                        .collect_vec();
                }

                Ok(())
            },
        )?;
        info!("computing...");
        Ok(())
    }

    /// Assigns dummy values to the regions created when calling `configure`.
    /// # Arguments
    /// * `input_shapes` - The shapes of the inputs to the model.
    pub fn dummy_layout(&self, input_shapes: &[Vec<usize>]) -> Result<usize, Box<dyn Error>> {
        info!("calculating num of constraints using dummy model layout...");
        let mut results = BTreeMap::<usize, ValTensor<F>>::new();

        let inputs: Vec<ValTensor<F>> = input_shapes
            .iter()
            .map(|shape| {
                let t: Tensor<Value<F>> = Tensor::new(None, shape).unwrap();
                t.into()
            })
            .collect_vec();

        for (i, input_idx) in self.inputs.iter().enumerate() {
            results.insert(*input_idx, inputs[i].clone());
        }

        let mut dummy_config = PolyConfig::dummy(self.run_args.logrows as usize);

        let mut offset: usize = 0;
        for (idx, node) in self.nodes.iter() {
            debug!(
                "dummy layout {}: {}, offset: {}",
                idx,
                node.opkind.as_str(),
                offset
            );

            let values: Vec<ValTensor<F>> = node
                .inputs
                .iter()
                .map(|i| results.get(i).unwrap().clone())
                .collect_vec();

            let res = dummy_config
                .layout(&mut None, &values, &mut offset, node.opkind.clone_dyn())
                .map_err(|e| {
                    error!("{}", e);
                    halo2_proofs::plonk::Error::Synthesis
                })?;

            if let Some(vt) = res {
                // we get the max as for fused nodes this corresponds to the node output
                results.insert(*idx, vt);
            }
        }

        let output_nodes = self.outputs.iter();
        debug!(
            "model outputs are nodes: {:?}",
            output_nodes.clone().collect_vec()
        );
        let mut outputs = output_nodes
            .map(|o| results.get(o).unwrap().clone())
            .collect_vec();

        // pack outputs if need be
        if self.run_args.pack_base > 1 {
            for i in 0..outputs.len() {
                debug!("packing outputs...");
                outputs[i] = dummy_config
                    .layout(
                        &mut None,
                        &outputs[i..i + 1],
                        &mut offset,
                        Box::new(PolyOp::Pack(self.run_args.pack_base, self.run_args.scale)),
                    )
                    .map_err(|e| {
                        error!("{}", e);
                        halo2_proofs::plonk::Error::Synthesis
                    })?
                    .unwrap();
            }
        }

        if self.run_args.public_outputs {
            let _ = outputs
                .into_iter()
                .map(|output| {
                    dummy_config.layout(
                        &mut None,
                        &[output.clone(), output],
                        &mut offset,
                        Box::new(PolyOp::RangeCheck(self.run_args.tolerance as i32)),
                    )
                })
                .collect_vec();
        }

        Ok(offset)
    }

    /// Returns the number of the computational graph's inputs
    pub fn num_inputs(&self) -> usize {
        let input_nodes = self.inputs.iter();
        input_nodes.len()
    }

    ///  Returns shapes of the computational graph's inputs
    pub fn input_shapes(&self) -> Vec<Vec<usize>> {
        self.inputs
            .iter()
            .map(|o| self.nodes.get(o).unwrap().out_dims.clone())
            .collect_vec()
    }

    /// Returns the number of the computational graph's outputs
    pub fn num_outputs(&self) -> usize {
        let output_nodes = self.outputs.iter();
        output_nodes.len()
    }

    /// Returns shapes of the computational graph's outputs
    pub fn output_shapes(&self) -> Vec<Vec<usize>> {
        self.outputs
            .iter()
            .map(|o| self.nodes.get(o).unwrap().out_dims.clone())
            .collect_vec()
    }

    /// Returns the fixed point scale of the computational graph's outputs
    pub fn get_output_scales(&self) -> Vec<u32> {
        let output_nodes = self.outputs.iter();
        output_nodes
            .map(|o| self.nodes.get(o).unwrap().out_scale)
            .collect_vec()
    }

    /// Number of instances used by the circuit
    pub fn instance_shapes(&self) -> Vec<Vec<usize>> {
        // for now the number of instances corresponds to the number of graph / model outputs
        let mut instance_shapes = vec![];
        if self.visibility.input.is_public() {
            instance_shapes.extend(self.input_shapes());
        }
        if self.visibility.output.is_public() {
            instance_shapes.extend(self.output_shapes());
        }
        instance_shapes
    }
}
