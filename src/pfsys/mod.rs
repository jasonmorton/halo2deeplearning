/// EVM related proving and verification
pub mod evm;

use crate::commands::{data_path, Cli};
use crate::fieldutils::{felt_to_i32, i32_to_felt};
use crate::graph::{utilities::vector_to_quantized, Model, ModelCircuit};
use crate::tensor::{Tensor, TensorType};
use halo2_proofs::arithmetic::FieldExt;
use halo2_proofs::circuit::Value;
use halo2_proofs::plonk::{
    create_proof, keygen_pk, keygen_vk, verify_proof, Circuit, ProvingKey, VerifyingKey,
};
use halo2_proofs::poly::commitment::{CommitmentScheme, Params, Prover, Verifier};
use halo2_proofs::poly::VerificationStrategy;
use halo2_proofs::transcript::{EncodedChallenge, TranscriptReadBuffer, TranscriptWriterBuffer};
use halo2curves::group::ff::PrimeField;
use halo2curves::serde::SerdeObject;
use halo2curves::CurveAffine;
use log::{info, trace};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use snark_verifier::system::halo2::{compile, Config};
use snark_verifier::verifier::plonk::PlonkProtocol;
use std::error::Error;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Cursor, Read, Write};
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

/// An application snark with proof and instance variables ready for aggregation (raw field element)
#[derive(Debug, Clone)]
pub struct Snark<F: FieldExt, C: CurveAffine> {
    protocol: Option<PlonkProtocol<C>>,
    /// public instances of the snark
    pub instances: Vec<Vec<F>>,
    proof: Vec<u8>,
}

impl<F: FieldExt, C: CurveAffine> Snark<F, C> {
    /// Create a new application snark from proof and instance variables ready for aggregation
    pub fn new(protocol: PlonkProtocol<C>, instances: Vec<Vec<F>>, proof: Vec<u8>) -> Self {
        Self {
            protocol: Some(protocol),
            instances,
            proof,
        }
    }
}

/// An application snark with proof and instance variables ready for aggregation (wrapped field element)
#[derive(Clone, Debug)]
pub struct SnarkWitness<F: FieldExt, C: CurveAffine> {
    protocol: PlonkProtocol<C>,
    instances: Vec<Vec<Value<F>>>,
    proof: Value<Vec<u8>>,
}

impl<F: FieldExt, C: CurveAffine> SnarkWitness<F, C> {
    fn without_witnesses(&self) -> Self {
        SnarkWitness {
            protocol: self.protocol.clone(),
            instances: self
                .instances
                .iter()
                .map(|instances| vec![Value::unknown(); instances.len()])
                .collect(),
            proof: Value::unknown(),
        }
    }

    fn proof(&self) -> Value<&[u8]> {
        self.proof.as_ref().map(Vec::as_slice)
    }
}

impl<F: FieldExt, C: CurveAffine> From<Snark<F, C>> for SnarkWitness<F, C> {
    fn from(snark: Snark<F, C>) -> Self {
        Self {
            protocol: snark.protocol.unwrap(),
            instances: snark
                .instances
                .into_iter()
                .map(|instances| instances.into_iter().map(Value::known).collect())
                .collect(),
            proof: Value::known(snark.proof),
        }
    }
}

/// Defines the proof generated by a model / circuit suitably for serialization/deserialization.  
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Snarki32 {
    /// Public inputs to the model.
    pub instances: Vec<Vec<i32>>,
    /// The generated proof, as a vector of bytes.
    pub proof: Vec<u8>,
}

impl<F: FieldExt, C: CurveAffine> Snark<F, C> {
    /// Saves the Proof to a specified `proof_path`.
    pub fn save(&self, proof_path: &PathBuf) -> Result<(), Box<dyn Error>> {
        let self_i32 = Snarki32 {
            instances: self
                .instances
                .iter()
                .map(|i| i.iter().map(|e| felt_to_i32::<F>(*e)).collect::<Vec<i32>>())
                .collect::<Vec<Vec<i32>>>(),
            proof: self.proof.clone(),
        };

        let serialized = serde_json::to_string(&self_i32).map_err(Box::<dyn Error>::from)?;

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
        let proof_i32: Snarki32 = serde_json::from_str(&data).map_err(Box::<dyn Error>::from)?;
        Ok(Snark {
            protocol: None,
            instances: proof_i32
                .instances
                .iter()
                .map(|i| i.iter().map(|e| i32_to_felt::<F>(*e)).collect::<Vec<F>>())
                .collect::<Vec<Vec<F>>>(),
            proof: proof_i32.proof,
        })
    }
}

type CircuitInputs<F> = (ModelCircuit<F>, Vec<Vec<F>>);

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

    let pi_inner: Vec<Vec<F>> = public_inputs
        .iter()
        .map(|i| i.iter().map(|e| i32_to_felt::<F>(*e)).collect::<Vec<F>>())
        .collect::<Vec<Vec<F>>>();

    Ok((circuit, pi_inner))
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
pub fn create_keys<Scheme: CommitmentScheme, F: FieldExt + TensorType, C: Circuit<F>>(
    circuit: &C,
    params: &'_ Scheme::ParamsProver,
) -> Result<ProvingKey<Scheme::Curve>, halo2_proofs::plonk::Error>
where
    C: Circuit<Scheme::Scalar>,
{
    //	Real proof
    let empty_circuit = <C as Circuit<F>>::without_witnesses(circuit);

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
    C: Circuit<F>,
    P: Prover<'params, Scheme>,
    V: Verifier<'params, Scheme>,
    Strategy: VerificationStrategy<'params, Scheme, V>,
    E: EncodedChallenge<Scheme::Curve>,
    TW: TranscriptWriterBuffer<Vec<u8>, Scheme::Curve, E>,
    TR: TranscriptReadBuffer<Cursor<Vec<u8>>, Scheme::Curve, E>,
>(
    circuit: C,
    public_inputs: Vec<Vec<Scheme::Scalar>>,
    params: &'params Scheme::ParamsProver,
    verifier_params: &'params Scheme::ParamsVerifier,
    pk: &ProvingKey<Scheme::Curve>,
    strategy: Strategy,
) -> Result<Snark<Scheme::Scalar, Scheme::Curve>, halo2_proofs::plonk::Error>
where
    C: Circuit<Scheme::Scalar>,
{
    let now = Instant::now();
    let mut transcript = TranscriptWriterBuffer::<_, Scheme::Curve, _>::init(vec![]);
    let mut rng = OsRng;

    let number_instance = public_inputs.iter().map(|x| x.len()).collect();
    trace!("number_instance {:?}", number_instance);
    let protocol = compile(
        params,
        pk.get_vk(),
        Config::kzg().with_num_instance(number_instance),
    );

    let pi_inner = public_inputs
        .iter()
        .map(|e| e.deref())
        .collect::<Vec<&[Scheme::Scalar]>>();
    let instances: &[&[&[Scheme::Scalar]]] = &[&pi_inner];
    trace!("instances {:?}", instances);

    create_proof::<Scheme, P, _, _, TW, _>(
        params,
        pk,
        &[circuit],
        instances,
        &mut rng,
        &mut transcript,
    )?;
    let proof = transcript.finalize();
    info!("Proof took {}", now.elapsed().as_secs());

    let checkable_pf = Snark {
        protocol: Some(protocol),
        instances: public_inputs
            .iter()
            .map(|i| i.clone().into_iter().collect())
            .collect(),
        proof,
    };

    {
        verify_proof_model::<F, V, Scheme, Strategy, E, TR>(
            checkable_pf.clone(),
            &verifier_params,
            pk.get_vk(),
            strategy,
        )?;
    }

    Ok(checkable_pf)
}

/// A wrapper around halo2's verify_proof
pub fn verify_proof_model<
    'params,
    F: FieldExt,
    V: Verifier<'params, Scheme>,
    Scheme: CommitmentScheme,
    Strategy: VerificationStrategy<'params, Scheme, V>,
    E: EncodedChallenge<Scheme::Curve>,
    TR: TranscriptReadBuffer<Cursor<Vec<u8>>, Scheme::Curve, E>,
>(
    snark: Snark<Scheme::Scalar, Scheme::Curve>,
    params: &'params Scheme::ParamsVerifier,
    vk: &VerifyingKey<Scheme::Curve>,
    strategy: Strategy,
) -> Result<Strategy::Output, halo2_proofs::plonk::Error> {
    let pi_inner = snark
        .instances
        .iter()
        .map(|e| e.deref())
        .collect::<Vec<&[Scheme::Scalar]>>();
    let instances: &[&[&[Scheme::Scalar]]] = &[&pi_inner];
    trace!("instances {:?}", instances);

    let now = Instant::now();
    let mut transcript = TranscriptReadBuffer::init(Cursor::new(snark.proof.clone()));
    info!("verify took {}", now.elapsed().as_secs());
    verify_proof::<Scheme, V, _, TR, _>(params, vk, strategy, instances, &mut transcript)
}

/// Loads a [VerifyingKey] at `path`.
pub fn load_vk<Scheme: CommitmentScheme, F: FieldExt + TensorType>(
    path: PathBuf,
) -> Result<VerifyingKey<Scheme::Curve>, Box<dyn Error>>
where
    ModelCircuit<F>: Circuit<Scheme::Scalar>,
    Scheme::Curve: SerdeObject + CurveAffine,
    Scheme::Scalar: PrimeField + SerdeObject,
{
    info!("loading verification key from {:?}", path);
    let f = File::open(path).map_err(Box::<dyn Error>::from)?;
    let mut reader = BufReader::new(f);
    VerifyingKey::<Scheme::Curve>::read::<_, ModelCircuit<F>>(
        &mut reader,
        halo2_proofs::SerdeFormat::Processed,
    )
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
) -> Result<(), io::Error>
where
    Scheme::Curve: SerdeObject + CurveAffine,
    Scheme::Scalar: PrimeField + SerdeObject,
{
    info!("saving verification key 💾");
    let f = File::create(path)?;
    let mut writer = BufWriter::new(f);
    vk.write(&mut writer, halo2_proofs::SerdeFormat::Processed)?;
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
