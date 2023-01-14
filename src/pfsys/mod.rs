/// Aggregation circuit
#[cfg(feature = "evm")]
pub mod aggregation;

use crate::commands::{data_path, Cli};
use crate::fieldutils::i32_to_felt;
use crate::graph::{utilities::vector_to_quantized, Model, ModelCircuit};
use crate::tensor::{Tensor, TensorType};
use halo2_proofs::arithmetic::FieldExt;
use halo2_proofs::plonk::{
    create_proof, keygen_pk, keygen_vk, verify_proof, Circuit, ProvingKey, VerifyingKey,
};
use halo2_proofs::poly::commitment::{CommitmentScheme, Params, Prover, Verifier};
use halo2_proofs::poly::VerificationStrategy;
use halo2_proofs::transcript::{
    Blake2bRead, Blake2bWrite, Challenge255, TranscriptReadBuffer, TranscriptWriterBuffer,
};
use log::{info, trace};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::marker::PhantomData;
use std::ops::Deref;
use std::path::PathBuf;
use std::time::Instant;

/// The input tensor data and shape, and output data for the computational graph (model) as floats.
/// For example, the input might be the image data for a neural network, and the output class scores.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelInput {
    /// Inputs to the model / computational graph.
    pub input_data: Vec<Vec<f32>>,
    /// The shape of said inputs.
    pub input_shapes: Vec<Vec<usize>>,
    /// The expected output of the model (can be empty vectors if outputs are not being constrained).
    pub output_data: Vec<Vec<f32>>,
}

/// Defines the proof generated by a model / circuit suitably for serialization/deserialization.  
#[derive(Debug, Deserialize, Serialize)]
pub struct Proof {
    /// Public inputs to the model.
    pub public_inputs: Vec<Vec<i32>>,
    /// The generated proof, as a vector of bytes.
    pub proof: Vec<u8>,
}

impl Proof {
    /// Saves the Proof to a specified `proof_path`.
    pub fn save(&self, proof_path: &PathBuf) -> Result<(), Box<dyn Error>> {
        let serialized = serde_json::to_string(&self).map_err(Box::<dyn Error>::from)?;

        let mut file = std::fs::File::create(proof_path).map_err(Box::<dyn Error>::from)?;
        file.write_all(serialized.as_bytes())
            .map_err(Box::<dyn Error>::from)
    }

    /// Load a json serialized proof from the provided path.
    pub fn load(proof_path: &PathBuf) -> Result<Self, Box<dyn Error>> {
        let mut file = File::open(proof_path).map_err(Box::<dyn Error>::from)?;
        let mut data = String::new();
        file.read_to_string(&mut data)
            .map_err(Box::<dyn Error>::from)?;
        serde_json::from_str(&data).map_err(Box::<dyn Error>::from)
    }
}

type CircuitInputs<F> = (ModelCircuit<F>, Vec<Tensor<i32>>);

/// Initialize the model circuit and quantize the provided float inputs from the provided `ModelInput`.
pub fn prepare_circuit_and_public_input<F: FieldExt>(
    data: &ModelInput,
    args: &Cli,
) -> Result<CircuitInputs<F>, Box<dyn Error>> {
    let model = Model::from_ezkl_conf(args.clone())?;
    let out_scales = model.get_output_scales();
    let circuit = prepare_circuit(data, args)?;

    // quantize the supplied data using the provided scale.
    // the ordering here is important, we want the inputs to come before the outputs
    // as they are configured in that order as Column<Instances>
    let mut public_inputs = vec![];
    if model.visibility.input.is_public() {
        for v in data.input_data.iter() {
            let t = vector_to_quantized(v, &Vec::from([v.len()]), 0.0, model.scale)?;
            public_inputs.push(t);
        }
    }
    if model.visibility.output.is_public() {
        for (idx, v) in data.output_data.iter().enumerate() {
            let t = vector_to_quantized(v, &Vec::from([v.len()]), 0.0, out_scales[idx])?;
            public_inputs.push(t);
        }
    }
    info!(
        "public inputs lengths: {:?}",
        public_inputs
            .iter()
            .map(|i| i.len())
            .collect::<Vec<usize>>()
    );
    trace!("{:?}", public_inputs);

    Ok((circuit, public_inputs))
}

/// Initialize the model circuit
pub fn prepare_circuit<F: FieldExt>(
    data: &ModelInput,
    args: &Cli,
) -> Result<ModelCircuit<F>, Box<dyn Error>> {
    // quantize the supplied data using the provided scale.
    let mut inputs: Vec<Tensor<i32>> = vec![];
    for (input, shape) in data.input_data.iter().zip(data.input_shapes.clone()) {
        let t = vector_to_quantized(input, &shape, 0.0, args.scale)?;
        inputs.push(t);
    }

    Ok(ModelCircuit::<F> {
        inputs,
        _marker: PhantomData,
    })
}

/// Deserializes the required inputs to a model at path `datapath` to a [ModelInput] struct.
pub fn prepare_data(datapath: String) -> Result<ModelInput, Box<dyn Error>> {
    let mut file = File::open(data_path(datapath)).map_err(Box::<dyn Error>::from)?;
    let mut data = String::new();
    file.read_to_string(&mut data)
        .map_err(Box::<dyn Error>::from)?;
    serde_json::from_str(&data).map_err(Box::<dyn Error>::from)
}

/// Creates a [VerifyingKey] and [ProvingKey] for a [ModelCircuit] (`circuit`) with specific [CommitmentScheme] parameters (`params`).
pub fn create_keys<Scheme: CommitmentScheme, F: FieldExt + TensorType>(
    circuit: &ModelCircuit<F>,
    params: &'_ Scheme::ParamsProver,
) -> Result<ProvingKey<Scheme::Curve>, halo2_proofs::plonk::Error>
where
    ModelCircuit<F>: Circuit<Scheme::Scalar>,
{
    //	Real proof
    let empty_circuit = circuit.without_witnesses();

    // Initialize the proving key
    let now = Instant::now();
    trace!("preparing VK");
    let vk = keygen_vk(params, &empty_circuit)?;
    info!("VK took {}", now.elapsed().as_secs());
    let now = Instant::now();
    let pk = keygen_pk(params, vk, &empty_circuit)?;
    info!("PK took {}", now.elapsed().as_secs());
    Ok(pk)
}

/// a wrapper around halo2's create_proof
pub fn create_proof_model<
    'params,
    Scheme: CommitmentScheme,
    F: FieldExt + TensorType,
    P: Prover<'params, Scheme>,
>(
    circuit: &ModelCircuit<F>,
    public_inputs: &[Tensor<i32>],
    params: &'params Scheme::ParamsProver,
    pk: &ProvingKey<Scheme::Curve>,
) -> Result<(Proof, Vec<Vec<usize>>), halo2_proofs::plonk::Error>
where
    ModelCircuit<F>: Circuit<Scheme::Scalar>,
{
    let now = Instant::now();
    let mut transcript = Blake2bWrite::<_, Scheme::Curve, Challenge255<_>>::init(vec![]);
    let mut rng = OsRng;
    let pi_inner: Vec<Vec<Scheme::Scalar>> = public_inputs
        .iter()
        .map(|i| {
            i.iter()
                .map(|e| i32_to_felt::<Scheme::Scalar>(*e))
                .collect::<Vec<Scheme::Scalar>>()
        })
        .collect::<Vec<Vec<Scheme::Scalar>>>();
    let pi_inner = pi_inner
        .iter()
        .map(|e| e.deref())
        .collect::<Vec<&[Scheme::Scalar]>>();
    let instances: &[&[&[Scheme::Scalar]]] = &[&pi_inner];
    trace!("instances {:?}", instances);

    let dims = circuit.inputs.iter().map(|i| i.dims().to_vec()).collect();

    create_proof::<Scheme, P, _, _, _, _>(
        params,
        pk,
        &[circuit.clone()],
        instances,
        &mut rng,
        &mut transcript,
    )?;
    let proof = transcript.finalize();
    info!("Proof took {}", now.elapsed().as_secs());

    let checkable_pf = Proof {
        public_inputs: public_inputs
            .iter()
            .map(|i| i.clone().into_iter().collect())
            .collect(),
        proof,
    };

    Ok((checkable_pf, dims))
}

/// A wrapper around halo2's verify_proof
pub fn verify_proof_model<
    'params,
    F: FieldExt,
    V: Verifier<'params, Scheme>,
    Scheme: CommitmentScheme,
    Strategy: VerificationStrategy<'params, Scheme, V>,
>(
    proof: Proof,
    params: &'params Scheme::ParamsVerifier,
    vk: &VerifyingKey<Scheme::Curve>,
    strategy: Strategy,
) -> Result<Strategy::Output, halo2_proofs::plonk::Error>
where
    ModelCircuit<F>: Circuit<Scheme::Scalar>,
{
    let pi_inner: Vec<Vec<Scheme::Scalar>> = proof
        .public_inputs
        .iter()
        .map(|i| {
            i.iter()
                .map(|e| i32_to_felt::<Scheme::Scalar>(*e))
                .collect::<Vec<Scheme::Scalar>>()
        })
        .collect::<Vec<Vec<Scheme::Scalar>>>();
    let pi_inner = pi_inner
        .iter()
        .map(|e| e.deref())
        .collect::<Vec<&[Scheme::Scalar]>>();
    let instances: &[&[&[Scheme::Scalar]]] = &[&pi_inner];
    trace!("instances {:?}", instances);

    let now = Instant::now();
    let mut transcript = Blake2bRead::<_, _, Challenge255<_>>::init(&proof.proof[..]);
    info!("verify took {}", now.elapsed().as_secs());
    verify_proof::<Scheme, V, _, _, _>(params, vk, strategy, instances, &mut transcript)
}

/// Loads a [VerifyingKey] at `path`.
pub fn load_vk<Scheme: CommitmentScheme, F: FieldExt + TensorType>(
    path: PathBuf,
    params: &'_ Scheme::ParamsVerifier,
) -> Result<VerifyingKey<Scheme::Curve>, Box<dyn Error>>
where
    ModelCircuit<F>: Circuit<Scheme::Scalar>,
{
    info!("loading verification key from {:?}", path);
    let f = File::open(path).map_err(Box::<dyn Error>::from)?;
    let mut reader = BufReader::new(f);
    VerifyingKey::<Scheme::Curve>::read::<_, ModelCircuit<F>>(&mut reader, params)
        .map_err(Box::<dyn Error>::from)
}

/// Loads the [CommitmentScheme::ParamsVerifier] at `path`.
pub fn load_params<Scheme: CommitmentScheme>(
    path: PathBuf,
) -> Result<Scheme::ParamsVerifier, Box<dyn Error>> {
    info!("loading params from {:?}", path);
    let f = File::open(path).map_err(Box::<dyn Error>::from)?;
    let mut reader = BufReader::new(f);
    Params::<'_, Scheme::Curve>::read(&mut reader).map_err(Box::<dyn Error>::from)
}

/// Saves a [VerifyingKey] to `path`.
pub fn save_vk<Scheme: CommitmentScheme>(
    path: &PathBuf,
    vk: &VerifyingKey<Scheme::Curve>,
) -> Result<(), io::Error> {
    info!("saving verification key 💾");
    let f = File::create(path)?;
    let mut writer = BufWriter::new(f);
    vk.write(&mut writer)?;
    writer.flush()?;
    Ok(())
}

/// Saves [CommitmentScheme] parameters to `path`.
pub fn save_params<Scheme: CommitmentScheme>(
    path: &PathBuf,
    params: &'_ Scheme::ParamsVerifier,
) -> Result<(), io::Error> {
    info!("saving parameters 💾");
    let f = File::create(path)?;
    let mut writer = BufWriter::new(f);
    params.write(&mut writer)?;
    writer.flush()?;
    Ok(())
}
