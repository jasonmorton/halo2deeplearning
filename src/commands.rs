use clap::{Args, Parser, Subcommand, ValueEnum};
#[cfg(not(target_arch = "wasm32"))]
use ethers::types::H160;
#[cfg(feature = "python-bindings")]
use pyo3::{
    conversion::{FromPyObject, PyTryFrom},
    exceptions::PyValueError,
    prelude::*,
    types::PyString,
};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::path::PathBuf;

use crate::circuit::{CheckMode, Tolerance};
#[cfg(not(target_arch = "wasm32"))]
use crate::graph::TestDataSource;
use crate::graph::Visibility;
use crate::pfsys::TranscriptType;

impl std::fmt::Display for TranscriptType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.to_possible_value()
            .expect("no values are skipped")
            .get_name()
            .fmt(f)
    }
}
#[cfg(feature = "python-bindings")]
/// Converts TranscriptType into a PyObject (Required for TranscriptType to be compatible with Python)
impl IntoPy<PyObject> for TranscriptType {
    fn into_py(self, py: Python) -> PyObject {
        match self {
            TranscriptType::Blake => "blake".to_object(py),
            TranscriptType::Poseidon => "poseidon".to_object(py),
            TranscriptType::EVM => "evm".to_object(py),
        }
    }
}
#[cfg(feature = "python-bindings")]
/// Obtains TranscriptType from PyObject (Required for TranscriptType to be compatible with Python)
impl<'source> FromPyObject<'source> for TranscriptType {
    fn extract(ob: &'source PyAny) -> PyResult<Self> {
        let trystr = <PyString as PyTryFrom>::try_from(ob)?;
        let strval = trystr.to_string();
        match strval.to_lowercase().as_str() {
            "blake" => Ok(TranscriptType::Blake),
            "poseidon" => Ok(TranscriptType::Poseidon),
            "evm" => Ok(TranscriptType::EVM),
            _ => Err(PyValueError::new_err("Invalid value for TranscriptType")),
        }
    }
}

#[allow(missing_docs)]
#[derive(ValueEnum, Copy, Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum StrategyType {
    Single,
    Accum,
}
impl std::fmt::Display for StrategyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.to_possible_value()
            .expect("no values are skipped")
            .get_name()
            .fmt(f)
    }
}
#[cfg(feature = "python-bindings")]
/// Converts StrategyType into a PyObject (Required for StrategyType to be compatible with Python)
impl IntoPy<PyObject> for StrategyType {
    fn into_py(self, py: Python) -> PyObject {
        match self {
            StrategyType::Single => "single".to_object(py),
            StrategyType::Accum => "accum".to_object(py),
        }
    }
}
#[cfg(feature = "python-bindings")]
/// Obtains StrategyType from PyObject (Required for StrategyType to be compatible with Python)
impl<'source> FromPyObject<'source> for StrategyType {
    fn extract(ob: &'source PyAny) -> PyResult<Self> {
        let trystr = <PyString as PyTryFrom>::try_from(ob)?;
        let strval = trystr.to_string();
        match strval.to_lowercase().as_str() {
            "single" => Ok(StrategyType::Single),
            "accum" => Ok(StrategyType::Accum),
            _ => Err(PyValueError::new_err("Invalid value for StrategyType")),
        }
    }
}

#[derive(clap::ValueEnum, Debug, Default, Copy, Clone, Serialize, Deserialize)]
/// Determines what the calibration pass should optimize for
pub enum CalibrationTarget {
    /// Optimizes for reducing cpu and memory usage
    #[default]
    Resources,
    /// Optimizes for numerical accuracy given the fixed point representation
    Accuracy,
}

impl From<&str> for CalibrationTarget {
    fn from(s: &str) -> Self {
        match s {
            "resources" => CalibrationTarget::Resources,
            "accuracy" => CalibrationTarget::Accuracy,
            _ => panic!("invalid calibration target"),
        }
    }
}

#[cfg(feature = "python-bindings")]
/// Converts CalibrationTarget into a PyObject (Required for CalibrationTarget to be compatible with Python)
impl IntoPy<PyObject> for CalibrationTarget {
    fn into_py(self, py: Python) -> PyObject {
        match self {
            CalibrationTarget::Resources => "resources".to_object(py),
            CalibrationTarget::Accuracy => "accuracy".to_object(py),
        }
    }
}

#[cfg(feature = "python-bindings")]
/// Obtains CalibrationTarget from PyObject (Required for CalibrationTarget to be compatible with Python)
impl<'source> FromPyObject<'source> for CalibrationTarget {
    fn extract(ob: &'source PyAny) -> PyResult<Self> {
        let trystr = <PyString as PyTryFrom>::try_from(ob)?;
        let strval = trystr.to_string();
        match strval.to_lowercase().as_str() {
            "resources" => Ok(CalibrationTarget::Resources),
            "accuracy" => Ok(CalibrationTarget::Accuracy),
            _ => Err(PyValueError::new_err("Invalid value for CalibrationTarget")),
        }
    }
}

/// Parameters specific to a proving run
#[derive(Debug, Copy, Args, Deserialize, Serialize, Clone, Default, PartialEq, PartialOrd)]
pub struct RunArgs {
    /// The tolerance for error on model outputs
    #[arg(short = 'T', long, default_value = "0")]
    pub tolerance: Tolerance,
    /// The denominator in the fixed point representation used when quantizing
    #[arg(short = 'S', long, default_value = "7")]
    pub scale: u32,
    /// The number of bits used in lookup tables
    #[arg(short = 'B', long, default_value = "16")]
    pub bits: usize,
    /// The log_2 number of rows
    #[arg(short = 'K', long, default_value = "17")]
    pub logrows: u32,
    /// The number of batches to split the input data into
    #[arg(long, default_value = "1")]
    pub batch_size: usize,
    /// Flags whether inputs are public, private, hashed
    #[arg(long, default_value = "private")]
    pub input_visibility: Visibility,
    /// Flags whether outputs are public, private, hashed
    #[arg(long, default_value = "public")]
    pub output_visibility: Visibility,
    /// Flags whether params are public, private, hashed
    #[arg(long, default_value = "private")]
    pub param_visibility: Visibility,
    /// the number of constraints the circuit might use. If not specified, this will be calculated using a 'dummy layout' pass.
    #[arg(long)]
    pub allocated_constraints: Option<usize>,
}
use lazy_static::lazy_static;

// if CARGO VERSION is 0.0.0 replace with "source - no compatibility guaranteed"
lazy_static! {
    /// The version of the ezkl library
    pub static ref VERSION: &'static str =  if env!("CARGO_PKG_VERSION") == "0.0.0" {
       "source - no compatibility guaranteed"
    } else {
        env!("CARGO_PKG_VERSION")
    };
}

#[allow(missing_docs)]
#[derive(Parser, Debug, Clone, Deserialize, Serialize)]
#[command(author, about, long_about = None)]
#[clap(version = *VERSION)]
pub struct Cli {
    #[command(subcommand)]
    #[allow(missing_docs)]
    pub command: Commands,
}

impl Cli {
    /// Export the ezkl configuration as json
    pub fn as_json(&self) -> Result<String, Box<dyn Error>> {
        let serialized = match serde_json::to_string(&self) {
            Ok(s) => s,
            Err(e) => {
                return Err(Box::new(e));
            }
        };
        Ok(serialized)
    }
    /// Parse an ezkl configuration from a json
    pub fn from_json(arg_json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(arg_json)
    }
}

#[allow(missing_docs)]
#[derive(Debug, Subcommand, Clone, Deserialize, Serialize)]
pub enum Commands {
    /// Loads model and prints model table
    #[command(arg_required_else_help = true)]
    Table {
        /// The path to the .onnx model file
        #[arg(short = 'M', long)]
        model: PathBuf,
        /// proving arguments
        #[clap(flatten)]
        args: RunArgs,
    },

    #[cfg(feature = "render")]
    /// Renders the model circuit to a .png file. For an overview of how to interpret these plots, see https://zcash.github.io/halo2/user/dev-tools.html
    #[command(arg_required_else_help = true)]
    RenderCircuit {
        /// The path to the .onnx model file
        #[arg(short = 'M', long)]
        model: PathBuf,
        /// Path to save the .png circuit render
        #[arg(short = 'O', long)]
        output: PathBuf,
        /// proving arguments
        #[clap(flatten)]
        args: RunArgs,
    },

    /// Generates the witness from an input file.
    #[command(arg_required_else_help = true)]
    GenWitness {
        /// The path to the .json data file
        #[arg(short = 'D', long)]
        data: PathBuf,
        /// The path to the compiled model file
        #[arg(short = 'M', long)]
        compiled_model: PathBuf,
        /// Path to the witness (public and private inputs) .json file
        #[arg(short = 'O', long, default_value = "witness.json")]
        output: PathBuf,
        /// Path to circuit_settings .json file to read in
        #[arg(short = 'S', long)]
        settings_path: PathBuf,
    },

    /// Produces the proving hyperparameters, from run-args
    #[command(arg_required_else_help = true)]
    GenSettings {
        /// The path to the .onnx model file
        #[arg(short = 'M', long)]
        model: PathBuf,
        /// Path to circuit_settings file to output
        #[arg(short = 'O', long, default_value = "settings.json")]
        settings_path: PathBuf,
        /// proving arguments
        #[clap(flatten)]
        args: RunArgs,
    },

    /// Calibrates the proving scale, lookup bits and logrows from a circuit settings file.
    #[cfg(not(target_arch = "wasm32"))]
    #[command(arg_required_else_help = true)]
    CalibrateSettings {
        /// The path to the .onnx model file
        #[arg(short = 'M', long)]
        model: PathBuf,
        /// Path to circuit_settings file to read in AND overwrite.
        #[arg(short = 'O', long, default_value = "settings.json")]
        settings_path: PathBuf,
        /// The path to the .json calibration data file.
        #[arg(short = 'D', long = "data")]
        data: PathBuf,
        #[arg(long = "target", default_value = "resources")]
        /// Target for calibration.
        target: CalibrationTarget,
    },

    /// Generates a dummy SRS
    #[command(name = "gen-srs", arg_required_else_help = true)]
    GenSrs {
        /// The path to output to the desired srs file
        #[arg(long, default_value = "kzg.srs")]
        srs_path: PathBuf,
        /// number of logrows to use for srs
        #[arg(long)]
        logrows: usize,
    },

    #[cfg(not(target_arch = "wasm32"))]
    /// Gets an SRS from a circuit settings file.
    #[command(name = "get-srs", arg_required_else_help = true)]
    GetSrs {
        /// The path to output to the desired srs file
        #[arg(long, default_value = "kzg.srs")]
        srs_path: PathBuf,
        /// Path to circuit_settings file to read in. Overrides logrows if specified.
        #[arg(short = 'S', long, default_value = None)]
        settings_path: Option<PathBuf>,
        /// Number of logrows to use for srs. To manually override the logrows, omit specifying the settings_path
        #[arg(long, default_value = None)]
        logrows: Option<u32>,
        /// Check mode for srs. verifies downloaded srs is valid. set to unsafe for speed.
        #[arg(long, default_value = "safe")]
        check: CheckMode,
    },
    /// Loads model and input and runs mock prover (for testing)
    #[command(arg_required_else_help = true)]
    Mock {
        /// The path to the .json witness file
        #[arg(short = 'W', long)]
        witness: PathBuf,
        /// The path to the .onnx model file
        #[arg(short = 'M', long)]
        model: PathBuf,
        /// circuit params path
        #[arg(short = 'S', long)]
        settings_path: PathBuf,
    },

    /// Mock aggregate proofs
    #[command(arg_required_else_help = true)]
    MockAggregate {
        /// The path to the snarks to aggregate over
        #[arg(long)]
        aggregation_snarks: Vec<PathBuf>,
        /// logrows used for aggregation circuit
        #[arg(long)]
        logrows: u32,
    },

    /// setup aggregation circuit :)
    #[command(arg_required_else_help = true)]
    SetupAggregate {
        /// The path to samples of snarks that will be aggregated over
        #[arg(long)]
        sample_snarks: Vec<PathBuf>,
        /// The path to save the desired verfication key file
        #[arg(long, default_value = "vk_aggr.key")]
        vk_path: PathBuf,
        /// The path to save the desired proving key file
        #[arg(long, default_value = "pk_aggr.key")]
        pk_path: PathBuf,
        /// The path to SRS
        #[arg(long)]
        srs_path: PathBuf,
        /// logrows used for aggregation circuit
        #[arg(long)]
        logrows: u32,
    },
    /// Aggregates proofs :)
    #[command(arg_required_else_help = true)]
    Aggregate {
        /// The path to the snarks to aggregate over
        #[arg(long)]
        aggregation_snarks: Vec<PathBuf>,
        /// The path to load the desired proving key file
        #[arg(long)]
        pk_path: PathBuf,
        /// The path to the desired output file
        #[arg(long, default_value = "proof_aggr.proof")]
        proof_path: PathBuf,
        /// The path to SRS
        #[arg(long)]
        srs_path: PathBuf,
        #[arg(
            long,
            require_equals = true,
            num_args = 0..=1,
            default_value_t = TranscriptType::EVM,
            value_enum
        )]
        transcript: TranscriptType,
        /// logrows used for aggregation circuit
        #[arg(long)]
        logrows: u32,
        /// run sanity checks during calculations (safe or unsafe)
        #[arg(long, default_value = "safe")]
        check_mode: CheckMode,
    },
    /// Compiles a circuit from onnx to a simplified graph (einsum + other ops) and parameters as sets of field elements
    #[command(arg_required_else_help = true)]
    CompileModel {
        /// The path to the .onnx model file
        #[arg(short = 'M', long)]
        model: PathBuf,
        /// The path to output the processed model
        #[arg(long)]
        compiled_model: PathBuf,
        /// The path to load circuit params from
        #[arg(short = 'S', long)]
        settings_path: PathBuf,
    },
    /// Creates pk and vk
    #[command(arg_required_else_help = true)]
    Setup {
        /// The path to the compiled model file
        #[arg(short = 'M', long)]
        compiled_model: PathBuf,
        /// The srs path
        #[arg(long)]
        srs_path: PathBuf,
        /// The path to output the verfication key file
        #[arg(long, default_value = "vk.key")]
        vk_path: PathBuf,
        /// The path to output the proving key file
        #[arg(long, default_value = "pk.key")]
        pk_path: PathBuf,
        /// The path to load circuit params from
        #[arg(short = 'S', long)]
        settings_path: PathBuf,
    },

    #[cfg(not(target_arch = "wasm32"))]
    /// Fuzzes the proof pipeline with random inputs, random parameters, and random keys
    #[command(arg_required_else_help = true)]
    Fuzz {
        /// The path to the .json witness file, which should include both the network input (possibly private) and the network output (public input to the proof)
        #[arg(short = 'W', long)]
        witness: PathBuf,
        /// The path to the processed model file
        #[arg(short = 'M', long)]
        compiled_model: PathBuf,
        #[arg(
            long,
            require_equals = true,
            num_args = 0..=1,
            default_value_t = TranscriptType::Blake,
            value_enum
        )]
        transcript: TranscriptType,
        /// proving arguments
        #[clap(flatten)]
        args: RunArgs,
        /// number of fuzz iterations
        #[arg(long, default_value = "10")]
        num_runs: usize,
        /// optional circuit params path (overrides any run args set)
        #[arg(short = 'S', long)]
        settings_path: Option<PathBuf>,
    },
    #[cfg(not(target_arch = "wasm32"))]
    SetupTestEVMData {
        /// The path to the .json data file, which should include both the network input (possibly private) and the network output (public input to the proof)
        #[arg(short = 'D', long)]
        data: PathBuf,
        /// The path to the compiled model file
        #[arg(short = 'M', long)]
        compiled_model: PathBuf,
        /// The path to load circuit params from
        #[arg(long)]
        settings_path: PathBuf,
        /// For testing purposes only. The optional path to the .json data file that will be generated that contains the OnChain data storage information
        /// derived from the file information in the data .json file.
        ///  Should include both the network input (possibly private) and the network output (public input to the proof)
        #[arg(short = 'T', long)]
        test_data: PathBuf,
        /// RPC URL for an Ethereum node, if None will use Anvil but WON'T persist state
        #[arg(short = 'U', long)]
        rpc_url: Option<String>,
        /// where does the input data come from
        #[arg(long, default_value = "on-chain")]
        input_source: TestDataSource,
        /// where does the output data come from
        #[arg(long, default_value = "on-chain")]
        output_source: TestDataSource,
    },

    #[cfg(not(target_arch = "wasm32"))]
    /// Loads model, data, and creates proof
    #[command(arg_required_else_help = true)]
    Prove {
        /// The path to the .json witness file, which should include both the network input (possibly private) and the network output (public input to the proof)
        #[arg(short = 'W', long)]
        witness: PathBuf,
        /// The path to the compiled model file
        #[arg(short = 'M', long)]
        compiled_model: PathBuf,
        /// The path to load the desired proving key file
        #[arg(long)]
        pk_path: PathBuf,
        /// The path to the desired output file
        #[arg(long, default_value = "proof.proof")]
        proof_path: PathBuf,
        /// The parameter path
        #[arg(long)]
        srs_path: PathBuf,
        #[arg(
            long,
            require_equals = true,
            num_args = 0..=1,
            default_value_t = TranscriptType::Blake,
            value_enum
        )]
        transcript: TranscriptType,
        /// The proving strategy
        #[arg(
            long,
            require_equals = true,
            num_args = 0..=1,
            default_value_t = StrategyType::Single,
            value_enum
        )]
        strategy: StrategyType,
        /// The path to load circuit params from
        #[arg(short = 'S', long)]
        settings_path: PathBuf,
        /// run sanity checks during calculations (safe or unsafe)
        #[arg(long, default_value = "safe")]
        check_mode: CheckMode,
    },
    #[cfg(not(target_arch = "wasm32"))]
    /// Creates an EVM verifier for a single proof
    #[command(name = "create-evm-verifier", arg_required_else_help = true)]
    CreateEVMVerifier {
        /// The path to load the desired params file
        #[arg(long)]
        srs_path: PathBuf,
        /// The path to load circuit settings from
        #[arg(short = 'S', long)]
        settings_path: PathBuf,
        /// The path to load the desired verfication key file
        #[arg(long)]
        vk_path: PathBuf,
        /// The path to output the Solidity code
        #[arg(long, default_value = "evm_deploy.sol")]
        sol_code_path: PathBuf,
        /// The path to output the Solidity verifier ABI
        #[arg(long, default_value = "verifier_abi.json")]
        abi_path: PathBuf,
    },
    #[cfg(not(target_arch = "wasm32"))]
    /// Creates an EVM verifier that attests to on-chain inputs for a single proof
    #[command(name = "create-evm-da-verifier", arg_required_else_help = true)]
    CreateEVMDataAttestationVerifier {
        /// The path to load the desired srs file from
        #[arg(long)]
        srs_path: PathBuf,
        /// The path to load circuit settings from
        #[arg(short = 'S', long)]
        settings_path: PathBuf,
        /// The path to load the desired verfication key file
        #[arg(long)]
        vk_path: PathBuf,
        /// The path to output the Solidity code
        #[arg(long, default_value = "evm_da_deploy.sol")]
        sol_code_path: PathBuf,
        /// The path to output the Solidity verifier ABI
        #[arg(long, default_value = "verifier_da_abi.json")]
        abi_path: PathBuf,
        /// The path to the .json data file, which should
        /// contain the necessary calldata and accoount addresses  
        /// needed need to read from all the on-chain
        /// view functions that return the data that the network
        /// ingests as inputs.
        #[arg(short = 'D', long)]
        data: PathBuf,
        // todo, optionally allow supplying proving key
    },

    #[cfg(not(target_arch = "wasm32"))]
    /// Creates an EVM verifier for an aggregate proof
    #[command(name = "create-evm-verifier-aggr", arg_required_else_help = true)]
    CreateEVMVerifierAggr {
        /// The path to load the desired srs file from
        #[arg(long)]
        srs_path: PathBuf,
        /// The path to output to load the desired verfication key file
        #[arg(long)]
        vk_path: PathBuf,
        /// The path to the Solidity code
        #[arg(long, default_value = "evm_deploy_aggr.sol")]
        sol_code_path: PathBuf,
        /// The path to output the Solidity verifier ABI
        #[arg(long, default_value = "verifier_aggr_abi.json")]
        abi_path: PathBuf,
        // aggregated circuit settings paths, used to calculate the number of instances in the aggregate proof
        #[arg(long)]
        aggregation_settings: Vec<PathBuf>,
    },
    /// Verifies a proof, returning accept or reject
    #[command(arg_required_else_help = true)]
    Verify {
        /// The path to load circuit params from
        #[arg(short = 'S', long)]
        settings_path: PathBuf,
        /// The path to the proof file
        #[arg(long)]
        proof_path: PathBuf,
        /// The path to output the desired verfication key file (optional)
        #[arg(long)]
        vk_path: PathBuf,
        /// The kzg srs path
        #[arg(long)]
        srs_path: PathBuf,
    },
    /// Verifies an aggregate proof, returning accept or reject
    #[command(arg_required_else_help = true)]
    VerifyAggr {
        /// The path to the proof file
        #[arg(long)]
        proof_path: PathBuf,
        /// The path to output the desired verfication key file (optional)
        #[arg(long)]
        vk_path: PathBuf,
        /// The srs path
        #[arg(long)]
        srs_path: PathBuf,
        /// logrows used for aggregation circuit
        #[arg(long)]
        logrows: u32,
    },
    #[cfg(not(target_arch = "wasm32"))]
    DeployEvmVerifier {
        /// The path to the Solidity code
        #[arg(long)]
        sol_code_path: PathBuf,
        /// RPC URL for an Ethereum node, if None will use Anvil but WON'T persist state
        #[arg(short = 'U', long)]
        rpc_url: Option<String>,
        #[arg(long, default_value = "contract.address")]
        /// The path to output the contract address
        addr_path: PathBuf,
    },
    #[cfg(not(target_arch = "wasm32"))]
    #[command(name = "deploy-evm-da-verifier", arg_required_else_help = true)]
    DeployEvmDataAttestationVerifier {
        /// The path to the .json data file, which should include both the network input (possibly private) and the network output (public input to the proof)
        #[arg(short = 'D', long)]
        data: PathBuf,
        /// The path to load circuit params from
        #[arg(long)]
        settings_path: PathBuf,
        /// The path to the Solidity code
        #[arg(long)]
        sol_code_path: PathBuf,
        /// RPC URL for an Ethereum node, if None will use Anvil but WON'T persist state
        #[arg(short = 'U', long)]
        rpc_url: Option<String>,
        #[arg(long, default_value = "contract_da.address")]
        /// The path to output the contract address
        addr_path: PathBuf,
    },
    #[cfg(not(target_arch = "wasm32"))]
    /// Verifies a proof using a local EVM executor, returning accept or reject
    #[command(name = "verify-evm", arg_required_else_help = true)]
    VerifyEVM {
        /// The path to the proof file
        #[arg(long)]
        proof_path: PathBuf,
        /// The path to verfier contract's address
        #[arg(long)]
        addr: H160,
        /// RPC URL for an Ethereum node, if None will use Anvil but WON'T persist state
        #[arg(short = 'U', long)]
        rpc_url: Option<String>,
        /// does the verifier use data attestation ?
        #[arg(long, default_value = "false")]
        data_attestation: bool,
    },

    /// Print the proof in hexadecimal
    #[command(name = "print-proof-hex", arg_required_else_help = true)]
    PrintProofHex {
        /// The path to the proof file
        #[arg(long)]
        proof_path: PathBuf,
    },
}
