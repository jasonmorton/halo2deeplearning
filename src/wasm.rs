use crate::circuit::modules::poseidon::spec::{PoseidonSpec, POSEIDON_RATE, POSEIDON_WIDTH};
use crate::circuit::modules::poseidon::PoseidonChip;
use crate::circuit::modules::Module;
use crate::graph::modules::POSEIDON_LEN_GRAPH;
use halo2_proofs::plonk::*;
use halo2_proofs::poly::commitment::{CommitmentScheme, ParamsProver};
use halo2_proofs::poly::kzg::{
    commitment::{KZGCommitmentScheme, ParamsKZG},
    strategy::SingleStrategy as KZGSingleStrategy,
};
use halo2curves::bn256::{Bn256, Fr, G1Affine};
use halo2curves::ff::{FromUniformBytes, PrimeField};

use crate::tensor::TensorType;
use wasm_bindgen::prelude::*;

use console_error_panic_hook;

pub use wasm_bindgen_rayon::init_thread_pool;

#[wasm_bindgen]
/// Initialize panic hook for wasm
pub fn init_panic_hook() {
    console_error_panic_hook::set_once();
}

use crate::execute::{create_proof_circuit_kzg, verify_proof_circuit_kzg};
use crate::graph::{GraphCircuit, GraphSettings};

/// Generate a poseidon hash in browser. Input message
#[wasm_bindgen]
pub fn poseidon_hash_wasm(message: wasm_bindgen::Clamped<Vec<u8>>) -> Vec<u8> {
    let message: Vec<Fr> = bincode::deserialize(&message[..]).unwrap();

    let output =
        PoseidonChip::<PoseidonSpec, POSEIDON_WIDTH, POSEIDON_RATE, POSEIDON_LEN_GRAPH>::run(
            message.clone(),
        )
        .unwrap();

    bincode::serialize(&output).unwrap()
}

/// Generate circuit params in browser
#[wasm_bindgen]
pub fn gen_circuit_settings_wasm(
    model_ser: wasm_bindgen::Clamped<Vec<u8>>,
    run_args_ser: wasm_bindgen::Clamped<Vec<u8>>,
) -> Vec<u8> {
    let run_args: crate::commands::RunArgs = bincode::deserialize(&run_args_ser[..]).unwrap();

    // Read in circuit
    let mut reader = std::io::BufReader::new(&model_ser[..]);
    let model = crate::graph::Model::new(&mut reader, run_args).unwrap();
    let circuit = GraphCircuit::new(model, run_args, crate::circuit::CheckMode::UNSAFE).unwrap();
    let circuit_settings = circuit.settings;
    serde_json::to_vec(&circuit_settings).unwrap()
}

/// Generate proving key in browser
#[wasm_bindgen]
pub fn gen_pk_wasm(
    circuit_ser: wasm_bindgen::Clamped<Vec<u8>>,
    params_ser: wasm_bindgen::Clamped<Vec<u8>>,
    circuit_settings_ser: wasm_bindgen::Clamped<Vec<u8>>,
) -> Vec<u8> {
    // Read in circuit params
    let circuit_settings: GraphSettings =
        serde_json::from_slice(&circuit_settings_ser[..]).unwrap();
    // Read in kzg params
    let mut reader = std::io::BufReader::new(&params_ser[..]);
    let params: ParamsKZG<Bn256> =
        halo2_proofs::poly::commitment::Params::<'_, G1Affine>::read(&mut reader).unwrap();
    // Read in circuit
    let mut circuit_reader = std::io::BufReader::new(&circuit_ser[..]);
    let model = crate::graph::Model::new(&mut circuit_reader, circuit_settings.run_args).unwrap();

    let circuit = GraphCircuit::new(
        model,
        circuit_settings.run_args,
        crate::circuit::CheckMode::UNSAFE,
    )
    .unwrap();

    // Create proving key
    let pk = create_keys_wasm::<KZGCommitmentScheme<Bn256>, Fr, GraphCircuit>(&circuit, &params)
        .map_err(Box::<dyn std::error::Error>::from)
        .unwrap();

    let mut serialized_pk = Vec::new();
    pk.write(&mut serialized_pk, halo2_proofs::SerdeFormat::RawBytes)
        .unwrap();

    serialized_pk
}

/// Generate verifying key in browser
#[wasm_bindgen]
pub fn gen_vk_wasm(
    pk: wasm_bindgen::Clamped<Vec<u8>>,
    circuit_settings_ser: wasm_bindgen::Clamped<Vec<u8>>,
) -> Vec<u8> {
    // Read in circuit params
    let circuit_settings: GraphSettings =
        serde_json::from_slice(&circuit_settings_ser[..]).unwrap();

    // Read in proving key
    let mut reader = std::io::BufReader::new(&pk[..]);
    let pk = ProvingKey::<G1Affine>::read::<_, GraphCircuit>(
        &mut reader,
        halo2_proofs::SerdeFormat::RawBytes,
        circuit_settings.clone(),
    )
    .unwrap();

    let vk = pk.get_vk();

    let mut serialized_vk = Vec::new();
    vk.write(&mut serialized_vk, halo2_proofs::SerdeFormat::RawBytes)
        .unwrap();

    serialized_vk
}

/// Verify proof in browser using wasm
#[wasm_bindgen]
pub fn verify_wasm(
    proof_js: wasm_bindgen::Clamped<Vec<u8>>,
    vk: wasm_bindgen::Clamped<Vec<u8>>,
    circuit_settings_ser: wasm_bindgen::Clamped<Vec<u8>>,
    params_ser: wasm_bindgen::Clamped<Vec<u8>>,
) -> bool {
    let mut reader = std::io::BufReader::new(&params_ser[..]);
    let params: ParamsKZG<Bn256> =
        halo2_proofs::poly::commitment::Params::<'_, G1Affine>::read(&mut reader).unwrap();

    let circuit_settings: GraphSettings =
        serde_json::from_slice(&circuit_settings_ser[..]).unwrap();

    let snark: crate::pfsys::Snark<Fr, G1Affine> = serde_json::from_slice(&proof_js[..]).unwrap();

    let mut reader = std::io::BufReader::new(&vk[..]);
    let vk = VerifyingKey::<G1Affine>::read::<_, GraphCircuit>(
        &mut reader,
        halo2_proofs::SerdeFormat::RawBytes,
        circuit_settings,
    )
    .unwrap();

    let strategy = KZGSingleStrategy::new(params.verifier_params());

    let result = verify_proof_circuit_kzg(params.verifier_params(), snark, &vk, strategy);

    if result.is_ok() {
        true
    } else {
        false
    }
}

/// Prove in browser using wasm
#[wasm_bindgen]
pub fn prove_wasm(
    witness: wasm_bindgen::Clamped<Vec<u8>>,
    pk: wasm_bindgen::Clamped<Vec<u8>>,
    circuit_ser: wasm_bindgen::Clamped<Vec<u8>>,
    circuit_settings_ser: wasm_bindgen::Clamped<Vec<u8>>,
    params_ser: wasm_bindgen::Clamped<Vec<u8>>,
) -> Vec<u8> {
    // read in kzg params
    let mut reader = std::io::BufReader::new(&params_ser[..]);
    let params: ParamsKZG<Bn256> =
        halo2_proofs::poly::commitment::Params::<'_, G1Affine>::read(&mut reader).unwrap();

    // read in model input
    let data: crate::graph::GraphWitness = serde_json::from_slice(&witness[..]).unwrap();

    // read in circuit params
    let circuit_settings: GraphSettings =
        serde_json::from_slice(&circuit_settings_ser[..]).unwrap();

    // read in proving key
    let mut reader = std::io::BufReader::new(&pk[..]);
    let pk = ProvingKey::<G1Affine>::read::<_, GraphCircuit>(
        &mut reader,
        halo2_proofs::SerdeFormat::RawBytes,
        circuit_settings.clone(),
    )
    .unwrap();

    // read in circuit
    let mut reader = std::io::BufReader::new(&circuit_ser[..]);
    let model = crate::graph::Model::new(&mut reader, circuit_settings.run_args).unwrap();

    let mut circuit = GraphCircuit::new(
        model,
        circuit_settings.run_args,
        crate::circuit::CheckMode::UNSAFE,
    )
    .unwrap();

    // prep public inputs
    circuit.load_graph_witness(&data).unwrap();
    let public_inputs = circuit.prepare_public_inputs(&data).unwrap();

    let strategy = KZGSingleStrategy::new(&params);
    let proof = create_proof_circuit_kzg(
        circuit,
        &params,
        public_inputs,
        &pk,
        crate::pfsys::TranscriptType::EVM,
        strategy,
        crate::circuit::CheckMode::UNSAFE,
    )
    .unwrap();

    serde_json::to_string(&proof).unwrap().into_bytes()
}

// HELPER FUNCTIONS

/// Creates a [VerifyingKey] and [ProvingKey] for a [GraphCircuit] (`circuit`) with specific [CommitmentScheme] parameters (`params`) for the WASM target
#[cfg(target_arch = "wasm32")]
pub fn create_keys_wasm<Scheme: CommitmentScheme, F: PrimeField + TensorType, C: Circuit<F>>(
    circuit: &C,
    params: &'_ Scheme::ParamsProver,
) -> Result<ProvingKey<Scheme::Curve>, halo2_proofs::plonk::Error>
where
    C: Circuit<Scheme::Scalar>,
    <Scheme as CommitmentScheme>::Scalar: FromUniformBytes<64>,
{
    //	Real proof
    let empty_circuit = <C as Circuit<F>>::without_witnesses(circuit);

    // Initialize the proving key
    let vk = keygen_vk(params, &empty_circuit)?;
    let pk = keygen_pk(params, vk, &empty_circuit)?;
    Ok(pk)
}
