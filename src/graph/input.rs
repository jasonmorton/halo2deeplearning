use crate::fieldutils::i128_to_felt;
use halo2curves::bn256::Fr as Fp;
#[cfg(feature = "python-bindings")]
use pyo3::prelude::*;
#[cfg(feature = "python-bindings")]
use pyo3::types::PyDict;
#[cfg(feature = "python-bindings")]
use pyo3::ToPyObject;
// use serde::de::{Visitor, MapAccess};
// use serde::de::{Visitor, MapAccess};
#[cfg(not(target_arch = "wasm32"))]
use crate::tensor::Tensor;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
// use std::collections::HashMap;
use std::io::Read;
// use std::collections::HashMap;

use super::quantize_float;
use super::{modules::ModuleForwardResult, GraphError};

type Decimals = u8;
type Call = String;
type RPCUrl = String;

///
#[derive(Clone, Debug, PartialOrd, PartialEq)]
pub enum FileSourceInner {
    /// Inner elements of inputs coming from a file
    Float(f64),
    /// Inner elements of inputs coming from a witness
    Field(Fp),
}

impl Serialize for FileSourceInner {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            FileSourceInner::Field(data) => field_to_vecu64(data).serialize(serializer),
            FileSourceInner::Float(data) => data.serialize(serializer),
        }
    }
}

// !!! ALWAYS USE JSON SERIALIZATION FOR GRAPH INPUT
// UNTAGGED ENUMS WONT WORK :( as highlighted here:
impl<'de> Deserialize<'de> for FileSourceInner {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let this_json: Box<serde_json::value::RawValue> = Deserialize::deserialize(deserializer)?;

        let first_try: Result<f64, _> = serde_json::from_str(this_json.get());

        if let Ok(t) = first_try {
            return Ok(FileSourceInner::Float(t));
        }
        let second_try: Result<[u64; 4], _> = serde_json::from_str(this_json.get());
        if let Ok(t) = second_try {
            return Ok(FileSourceInner::Field(Fp::from_raw(t)));
        }

        Err(serde::de::Error::custom(
            "failed to deserialize FileSourceInner",
        ))
    }
}

/// Elements of inputs coming from a file
pub type FileSource = Vec<Vec<FileSourceInner>>;

impl FileSourceInner {
    /// Create a new FileSourceInner
    pub fn new_float(f: f64) -> Self {
        FileSourceInner::Float(f)
    }
    /// Create a new FileSourceInner
    pub fn new_field(f: Fp) -> Self {
        FileSourceInner::Field(f)
    }

    /// Convert to a field element
    pub fn to_field(&self, scale: u32) -> Fp {
        match self {
            FileSourceInner::Float(f) => i128_to_felt(quantize_float(f, 0.0, scale).unwrap()),
            FileSourceInner::Field(f) => *f,
        }
    }
}

/// Inner elements of witness coming from a witness
pub type WitnessFileSource = Vec<Vec<Fp>>;

/// Inner elements of inputs/outputs coming from on-chain
#[derive(Clone, Debug, Deserialize, Serialize, Default, PartialOrd, PartialEq)]
pub struct OnChainSource {
    /// Vector of calls to accounts
    pub calls: Vec<CallsToAccount>,
    /// RPC url
    pub rpc: RPCUrl,
}

impl OnChainSource {
    /// Create a new OnChainSource
    pub fn new(calls: Vec<CallsToAccount>, rpc: RPCUrl) -> Self {
        OnChainSource { calls, rpc }
    }
}

impl OnChainSource {
    #[cfg(not(target_arch = "wasm32"))]
    /// Create dummy local on-chain data to test the OnChain data source
    pub async fn test_from_file_data(
        data: &WitnessFileSource,
        scales: Vec<u32>,
        shapes: Vec<Vec<usize>>,
        rpc: Option<&str>,
    ) -> Result<(Vec<Tensor<Fp>>, Self), Box<dyn std::error::Error>> {
        use crate::eth::{evm_quantize, read_on_chain_inputs, test_on_chain_data};
        use crate::graph::scale_to_multiplier;
        use itertools::Itertools;
        use log::debug;

        // Set up local anvil instance for reading on-chain data
        let (anvil, client) = crate::eth::setup_eth_backend(rpc).await?;

        let address = client.address();

        let scales: Vec<f64> = scales.into_iter().map(scale_to_multiplier).collect();

        // unquantize data
        let float_data = data
            .iter()
            .zip(scales.iter())
            .map(|(t, scale)| {
                t.iter()
                    .map(|e| ((crate::fieldutils::felt_to_i128(*e) as f64 / scale) as f32))
                    .collect_vec()
            })
            .collect::<Vec<Vec<f32>>>();

        let calls_to_accounts = test_on_chain_data(client.clone(), &float_data).await?;
        debug!("Calls to accounts: {:?}", calls_to_accounts);
        let inputs = read_on_chain_inputs(client.clone(), address, &calls_to_accounts).await?;
        debug!("Inputs: {:?}", inputs);

        let mut quantized_evm_inputs = vec![];

        let mut prev = 0;
        for (idx, i) in data.iter().enumerate() {
            quantized_evm_inputs.extend(
                evm_quantize(
                    client.clone(),
                    vec![scales[idx]; i.len()],
                    &(
                        inputs.0[prev..i.len()].to_vec(),
                        inputs.1[prev..i.len()].to_vec(),
                    ),
                )
                .await?,
            );
            prev += i.len();
        }

        // on-chain data has already been quantized at this point. Just need to reshape it and push into tensor vector
        let mut inputs: Vec<Tensor<Fp>> = vec![];
        for (input, shape) in vec![quantized_evm_inputs].iter().zip(shapes) {
            let mut t: Tensor<Fp> = input.iter().cloned().collect();
            t.reshape(&shape);
            inputs.push(t);
        }

        let used_rpc = rpc.unwrap_or(&anvil.endpoint()).to_string();

        // Fill the input_data field of the GraphInput struct
        Ok((
            inputs,
            OnChainSource::new(calls_to_accounts.clone(), used_rpc),
        ))
    }
}

/// Defines the view only calls to accounts to fetch the on-chain input data.
/// This data will be included as part of the first elements in the publicInputs
/// for the sol evm verifier and will be  verifyWithDataAttestation.sol
#[derive(Clone, Debug, Deserialize, Serialize, Default, PartialOrd, PartialEq)]
pub struct CallsToAccount {
    /// A vector of tuples, where index 0 of tuples
    /// are the byte strings representing the ABI encoded function calls to
    /// read the data from the address. This call must return a single
    /// elementary type (https://docs.soliditylang.org/en/v0.8.20/abi-spec.html#types).
    /// The second index of the tuple is the number of decimals for f32 conversion.
    /// We don't support dynamic types currently.
    pub call_data: Vec<(Call, Decimals)>,
    /// Address of the contract to read the data from.
    pub address: String,
}
/// Enum that defines source of the inputs/outputs to the EZKL model
#[derive(Clone, Debug, Serialize, PartialOrd, PartialEq)]
#[serde(untagged)]
pub enum DataSource {
    /// .json File data source.
    File(FileSource),
    /// On-chain data source. The first element is the calls to the account, and the second is the RPC url.
    OnChain(OnChainSource),
}
impl Default for DataSource {
    fn default() -> Self {
        DataSource::File(vec![vec![]])
    }
}

impl From<FileSource> for DataSource {
    fn from(data: FileSource) -> Self {
        DataSource::File(data)
    }
}

impl From<Vec<Vec<Fp>>> for DataSource {
    fn from(data: Vec<Vec<Fp>>) -> Self {
        DataSource::File(
            data.iter()
                .map(|e| e.iter().map(|e| FileSourceInner::Field(*e)).collect())
                .collect(),
        )
    }
}

impl From<Vec<Vec<f64>>> for DataSource {
    fn from(data: Vec<Vec<f64>>) -> Self {
        DataSource::File(
            data.iter()
                .map(|e| e.iter().map(|e| FileSourceInner::Float(*e)).collect())
                .collect(),
        )
    }
}

impl From<OnChainSource> for DataSource {
    fn from(data: OnChainSource) -> Self {
        DataSource::OnChain(data)
    }
}

// !!! ALWAYS USE JSON SERIALIZATION FOR GRAPH INPUT
// UNTAGGED ENUMS WONT WORK :( as highlighted here:
impl<'de> Deserialize<'de> for DataSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let this_json: Box<serde_json::value::RawValue> = Deserialize::deserialize(deserializer)?;

        let first_try: Result<FileSource, _> = serde_json::from_str(this_json.get());

        if let Ok(t) = first_try {
            return Ok(DataSource::File(t));
        }
        let second_try: Result<OnChainSource, _> = serde_json::from_str(this_json.get());
        if let Ok(t) = second_try {
            return Ok(DataSource::OnChain(t));
        }

        Err(serde::de::Error::custom("failed to deserialize DataSource"))
    }
}

/// Enum that defines source of the inputs/outputs to the EZKL model
#[derive(Clone, Debug, PartialOrd, PartialEq)]
pub enum WitnessSource {
    /// .json File data source.
    File(WitnessFileSource),
    /// On-chain data source. The first element is the calls to the account, and the second is the RPC url.
    OnChain(OnChainSource),
}
impl Default for WitnessSource {
    fn default() -> Self {
        WitnessSource::File(vec![vec![]])
    }
}

impl From<WitnessFileSource> for WitnessSource {
    fn from(data: WitnessFileSource) -> Self {
        WitnessSource::File(data)
    }
}

impl From<OnChainSource> for WitnessSource {
    fn from(data: OnChainSource) -> Self {
        WitnessSource::OnChain(data)
    }
}

// !!! ALWAYS USE JSON SERIALIZATION FOR GRAPH INPUT
// UNTAGGED ENUMS WONT WORK :( as highlighted here:
impl<'de> Deserialize<'de> for WitnessSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let this_json: Box<serde_json::value::RawValue> = Deserialize::deserialize(deserializer)?;

        let first_try: Result<Vec<Vec<[u64; 4]>>, _> = serde_json::from_str(this_json.get());

        if let Ok(t) = first_try {
            let t: Vec<Vec<Fp>> = t
                .iter()
                .map(|x| x.iter().map(|fp| Fp::from_raw(*fp)).collect())
                .collect();
            return Ok(WitnessSource::File(t));
        }

        let second_try: Result<OnChainSource, _> = serde_json::from_str(this_json.get());
        if let Ok(t) = second_try {
            return Ok(WitnessSource::OnChain(t));
        }

        Err(serde::de::Error::custom(
            "failed to deserialize WitnessSource",
        ))
    }
}

impl Serialize for WitnessSource {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            WitnessSource::File(source) => {
                let field_elems: Vec<Vec<[u64; 4]>> = source
                    .iter()
                    .map(|x| x.iter().map(|fp| field_to_vecu64(fp)).collect())
                    .collect::<Vec<_>>();
                field_elems.serialize(serializer)
            }
            WitnessSource::OnChain(source) => {
                // leave it untagged
                let mut state = serializer.serialize_struct("", 2)?;
                state.serialize_field("rpc", &source.rpc)?;
                state.serialize_field("calls", &source.calls)?;
                state.end()
            }
        }
    }
}

/// The input tensor data and shape, and output data for the computational graph (model) as floats.
/// For example, the input might be the image data for a neural network, and the output class scores.
#[derive(Clone, Debug, Deserialize, Default)]
pub struct GraphWitness {
    /// Inputs to the model / computational graph (can be empty vectors if inputs are coming from on-chain).
    /// TODO: Add retrieve from on-chain functionality
    pub input_data: WitnessSource,
    /// The expected output of the model (can be empty vectors if outputs are not being constrained).
    pub output_data: WitnessSource,
    /// Optional hashes of the inputs (can be None if there are no commitments). Wrapped as Option for backwards compatibility
    pub processed_inputs: Option<ModuleForwardResult>,
    /// Optional hashes of the params (can be None if there are no commitments). Wrapped as Option for backwards compatibility
    pub processed_params: Option<ModuleForwardResult>,
    /// Optional hashes of the outputs (can be None if there are no commitments). Wrapped as Option for backwards compatibility
    pub processed_outputs: Option<ModuleForwardResult>,
}

impl GraphWitness {
    ///
    pub fn new(input_data: WitnessSource, output_data: WitnessSource) -> Self {
        GraphWitness {
            input_data,
            output_data,
            processed_inputs: None,
            processed_params: None,
            processed_outputs: None,
        }
    }
    /// Load the model input from a file
    pub fn from_path(path: std::path::PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let mut file = std::fs::File::open(path)?;
        let mut data = String::new();
        file.read_to_string(&mut data)?;
        serde_json::from_str(&data).map_err(|e| e.into())
    }

    /// Save the model input to a file
    pub fn save(&self, path: std::path::PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        serde_json::to_writer(std::fs::File::create(path)?, &self).map_err(|e| e.into())
    }
}
/// Input to graph as a datasource
/// Always use JSON serialization for GraphInput. Seriously.
#[derive(Clone, Debug, Deserialize, Default, PartialEq)]
pub struct GraphInput {
    /// Inputs to the model / computational graph (can be empty vectors if inputs are coming from on-chain).
    pub input_data: DataSource,
}

impl GraphInput {
    ///
    pub fn new(input_data: DataSource) -> Self {
        GraphInput { input_data }
    }

    /// Load the model input from a file
    pub fn from_path(path: std::path::PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let mut file = std::fs::File::open(path)?;
        let mut data = String::new();
        file.read_to_string(&mut data)?;
        serde_json::from_str(&data).map_err(|e| e.into())
    }

    /// Save the model input to a file
    pub fn save(&self, path: std::path::PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        serde_json::to_writer(std::fs::File::create(path)?, &self).map_err(|e| e.into())
    }

    ///
    pub fn split_into_batches(
        &self,
        batch_size: usize,
        input_shapes: Vec<Vec<usize>>,
    ) -> Result<Vec<Self>, Box<dyn std::error::Error>> {
        // split input data into batches
        let mut batched_inputs = vec![];

        let iterable = match self {
            GraphInput {
                input_data: DataSource::File(data),
            } => data,
            _ => {
                todo!("on-chain data batching not implemented yet")
            }
        };

        for (i, input) in iterable.iter().enumerate() {
            // ensure the input is devenly divisible by batch_size
            if input.len() % batch_size != 0 {
                return Err(Box::new(GraphError::InvalidDims(
                    0,
                    "input data length must be evenly divisible by batch size".to_string(),
                )));
            }
            let input_size = input_shapes[i].clone().iter().product::<usize>();
            let mut batches = vec![];
            for batch in input.chunks(batch_size * input_size) {
                batches.push(batch.to_vec());
            }
            batched_inputs.push(batches);
        }
        // now merge all the batches for each input into a vector of batches
        // first assert each input has the same number of batches
        let num_batches = batched_inputs[0].len();
        for input in batched_inputs.iter() {
            assert_eq!(input.len(), num_batches);
        }
        // now merge the batches
        let mut input_batches = vec![];
        for i in 0..num_batches {
            let mut batch = vec![];
            for input in batched_inputs.iter() {
                batch.push(input[i].clone());
            }
            input_batches.push(DataSource::File(batch));
        }

        // create a new GraphWitness for each batch
        let batches = input_batches
            .into_iter()
            .map(GraphInput::new)
            .collect::<Vec<GraphInput>>();

        Ok(batches)
    }
}

#[cfg(feature = "python-bindings")]
use halo2curves::bn256::G1Affine;
use halo2curves::{ff::PrimeField, serde::SerdeObject};

// #[cfg(feature = "python-bindings")]
/// converts fp into Vec<u64>
fn field_to_vecu64<F: PrimeField + SerdeObject + Serialize>(fp: &F) -> [u64; 4] {
    let bytes = fp.to_repr();
    let bytes_first_u64 = u64::from_le_bytes(bytes.as_ref()[0..8][..].try_into().unwrap());
    let bytes_second_u64 = u64::from_le_bytes(bytes.as_ref()[8..16][..].try_into().unwrap());
    let bytes_third_u64 = u64::from_le_bytes(bytes.as_ref()[16..24][..].try_into().unwrap());
    let bytes_fourth_u64 = u64::from_le_bytes(bytes.as_ref()[24..32][..].try_into().unwrap());

    [
        bytes_first_u64,
        bytes_second_u64,
        bytes_third_u64,
        bytes_fourth_u64,
    ]
}

#[cfg(feature = "python-bindings")]
fn field_to_vecu64_montgomery<F: PrimeField + SerdeObject + Serialize>(fp: &F) -> [u64; 4] {
    let repr = serde_json::to_string(&fp).unwrap();
    let b: [u64; 4] = serde_json::from_str(&repr).unwrap();
    b
}

#[cfg(feature = "python-bindings")]
fn insert_poseidon_hash_pydict(pydict: &PyDict, poseidon_hash: &Vec<Fp>) {
    let poseidon_hash: Vec<[u64; 4]> = poseidon_hash
        .iter()
        .map(field_to_vecu64_montgomery)
        .collect();
    pydict.set_item("poseidon_hash", poseidon_hash).unwrap();
}

#[cfg(feature = "python-bindings")]
fn g1affine_to_pydict(g1affine_dict: &PyDict, g1affine: &G1Affine) {
    let g1affine_x = field_to_vecu64_montgomery(&g1affine.x);
    let g1affine_y = field_to_vecu64_montgomery(&g1affine.y);
    g1affine_dict.set_item("x", g1affine_x).unwrap();
    g1affine_dict.set_item("y", g1affine_y).unwrap();
}

#[cfg(feature = "python-bindings")]
use super::modules::ElGamalResult;
#[cfg(feature = "python-bindings")]
fn insert_elgamal_results_pydict(py: Python, pydict: &PyDict, elgamal_results: &ElGamalResult) {
    let results_dict = PyDict::new(py);
    let cipher_text: Vec<Vec<[u64; 4]>> = elgamal_results
        .ciphertexts
        .iter()
        .map(|v| {
            v.iter()
                .map(field_to_vecu64_montgomery)
                .collect::<Vec<[u64; 4]>>()
        })
        .collect::<Vec<Vec<[u64; 4]>>>();
    results_dict.set_item("ciphertexts", cipher_text).unwrap();

    let variables_dict = PyDict::new(py);
    let variables = &elgamal_results.variables;

    let r = field_to_vecu64_montgomery(&variables.r);
    variables_dict.set_item("r", r).unwrap();
    // elgamal secret key
    let sk = field_to_vecu64_montgomery(&variables.sk);
    variables_dict.set_item("sk", sk).unwrap();

    let pk_dict = PyDict::new(py);
    // elgamal public key
    g1affine_to_pydict(pk_dict, &variables.pk);
    variables_dict.set_item("pk", pk_dict).unwrap();

    let aux_generator_dict = PyDict::new(py);
    // elgamal aux generator used in ecc chip
    g1affine_to_pydict(aux_generator_dict, &variables.aux_generator);
    variables_dict
        .set_item("aux_generator", aux_generator_dict)
        .unwrap();

    // elgamal window size used in ecc chip
    variables_dict
        .set_item("window_size", variables.window_size)
        .unwrap();

    results_dict.set_item("variables", variables_dict).unwrap();

    pydict.set_item("elgamal", results_dict).unwrap();

    //elgamal
}

#[cfg(feature = "python-bindings")]
impl ToPyObject for CallsToAccount {
    fn to_object(&self, py: Python) -> PyObject {
        let dict = PyDict::new(py);
        dict.set_item("account", &self.address).unwrap();
        dict.set_item("call_data", &self.call_data).unwrap();
        dict.to_object(py)
    }
}

#[cfg(feature = "python-bindings")]
impl ToPyObject for DataSource {
    fn to_object(&self, py: Python) -> PyObject {
        match self {
            DataSource::File(data) => data.to_object(py),
            DataSource::OnChain(source) => {
                let dict = PyDict::new(py);
                dict.set_item("rpc_url", &source.rpc).unwrap();
                dict.set_item("calls_to_accounts", &source.calls).unwrap();
                dict.to_object(py)
            }
        }
    }
}

#[cfg(feature = "python-bindings")]
impl ToPyObject for FileSourceInner {
    fn to_object(&self, py: Python) -> PyObject {
        match self {
            FileSourceInner::Field(data) => field_to_vecu64(data).to_object(py),
            FileSourceInner::Float(data) => data.to_object(py),
        }
    }
}

#[cfg(feature = "python-bindings")]
impl ToPyObject for WitnessSource {
    fn to_object(&self, py: Python) -> PyObject {
        match self {
            WitnessSource::File(data) => {
                let field_elem: Vec<Vec<[u64; 4]>> = data
                    .iter()
                    .map(|x| x.iter().map(field_to_vecu64).collect())
                    .collect();
                field_elem.to_object(py)
            }
            WitnessSource::OnChain(source) => {
                let dict = PyDict::new(py);
                dict.set_item("rpc_url", &source.rpc).unwrap();
                dict.set_item("calls_to_accounts", &source.calls).unwrap();
                dict.to_object(py)
            }
        }
    }
}

#[cfg(feature = "python-bindings")]
impl ToPyObject for GraphWitness {
    fn to_object(&self, py: Python) -> PyObject {
        // Create a Python dictionary
        let dict = PyDict::new(py);
        let dict_inputs = PyDict::new(py);
        let dict_params = PyDict::new(py);
        let dict_outputs = PyDict::new(py);

        let input_data_mut = &self.input_data;
        let output_data_mut = &self.output_data;

        dict.set_item("input_data", &input_data_mut).unwrap();
        dict.set_item("output_data", &output_data_mut).unwrap();

        if let Some(processed_inputs) = &self.processed_inputs {
            //poseidon_hash
            if let Some(processed_inputs_poseidon_hash) = &processed_inputs.poseidon_hash {
                insert_poseidon_hash_pydict(&dict_inputs, processed_inputs_poseidon_hash);
            }
            if let Some(processed_inputs_elgamal) = &processed_inputs.elgamal {
                insert_elgamal_results_pydict(py, dict_inputs, processed_inputs_elgamal);
            }

            dict.set_item("processed_inputs", dict_inputs).unwrap();
        }

        if let Some(processed_params) = &self.processed_params {
            if let Some(processed_params_poseidon_hash) = &processed_params.poseidon_hash {
                insert_poseidon_hash_pydict(dict_params, processed_params_poseidon_hash);
            }
            if let Some(processed_params_elgamal) = &processed_params.elgamal {
                insert_elgamal_results_pydict(py, dict_params, processed_params_elgamal);
            }

            dict.set_item("processed_params", dict_params).unwrap();
        }

        if let Some(processed_outputs) = &self.processed_outputs {
            if let Some(processed_outputs_poseidon_hash) = &processed_outputs.poseidon_hash {
                insert_poseidon_hash_pydict(dict_outputs, processed_outputs_poseidon_hash);
            }
            if let Some(processed_outputs_elgamal) = &processed_outputs.elgamal {
                insert_elgamal_results_pydict(py, dict_outputs, processed_outputs_elgamal);
            }

            dict.set_item("processed_outputs", dict_outputs).unwrap();
        }

        dict.to_object(py)
    }
}

impl Serialize for GraphInput {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("GraphInput", 4)?;
        state.serialize_field("input_data", &self.input_data)?;
        state.end()
    }
}

impl Serialize for GraphWitness {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("GraphWitness", 4)?;
        state.serialize_field("input_data", &self.input_data)?;
        state.serialize_field("output_data", &self.output_data)?;

        if let Some(processed_inputs) = &self.processed_inputs {
            state.serialize_field("processed_inputs", &processed_inputs)?;
        }

        if let Some(processed_params) = &self.processed_params {
            state.serialize_field("processed_params", &processed_params)?;
        }

        if let Some(processed_outputs) = &self.processed_outputs {
            state.serialize_field("processed_outputs", &processed_outputs)?;
        }
        state.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // this is for backwards compatibility with the old format
    fn test_data_source_serialization_round_trip() {
        let source = DataSource::from(vec![vec![0.053_262_424, 0.074_970_566, 0.052_355_476]]);

        let serialized = serde_json::to_string(&source).unwrap();

        const JSON: &str = r#"[[0.053262424,0.074970566,0.052355476]]"#;

        assert_eq!(serialized, JSON);

        let expect = serde_json::from_str::<DataSource>(JSON)
            .map_err(|e| e.to_string())
            .unwrap();

        assert_eq!(expect, source);
    }

    #[test]
    // this is for backwards compatibility with the old format
    fn test_graph_input_serialization_round_trip() {
        let file = GraphInput::new(DataSource::from(vec![vec![
            0.05326242372393608,
            0.07497056573629379,
            0.05235547572374344,
        ]]));

        let serialized = serde_json::to_string(&file).unwrap();

        const JSON: &str =
            r#"{"input_data":[[0.05326242372393608,0.07497056573629379,0.05235547572374344]]}"#;

        assert_eq!(serialized, JSON);

        let graph_input3 = serde_json::from_str::<GraphInput>(JSON)
            .map_err(|e| e.to_string())
            .unwrap();
        assert_eq!(graph_input3, file);
    }

    //  test for the compatibility with the serialized elements from the mclbn256 library
    #[test]
    fn test_python_compat() {
        let source = Fp::from_raw([18445520602771460712, 838677322461845011, 3079992810, 0]);

        let original_addr = "0x000000000000000000000000b794f5ea0ba39494ce839613fffba74279579268";

        assert_eq!(format!("{:?}", source), original_addr);
    }
}
