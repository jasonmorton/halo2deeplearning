use crate::circuit::{CheckMode, Tolerance};
use crate::commands::{RunArgs, StrategyType};
use crate::eth::{fix_verifier_sol, verify_proof_via_solidity};
use crate::execute::{create_proof_circuit_kzg, load_params_cmd, verify_proof_circuit_kzg};
use crate::graph::{quantize_float, Mode, Model, ModelCircuit, ModelParams, VarVisibility};
use crate::pfsys::evm::{evm_verify, single::gen_evm_verifier, DeploymentCode};
use crate::pfsys::{
    create_keys, gen_srs as ezkl_gen_srs, load_pk, load_vk, prepare_data, save_params, save_pk,
    save_vk, Snark, TranscriptType,
};
use halo2_proofs::poly::kzg::{
    commitment::KZGCommitmentScheme,
    strategy::{AccumulatorStrategy, SingleStrategy as KZGSingleStrategy},
};
use halo2_proofs::{dev::MockProver, poly::commitment::ParamsProver};
use halo2curves::bn256::{Bn256, Fr};
use log::trace;
use pyo3::exceptions::{PyIOError, PyRuntimeError};
use pyo3::prelude::*;
use pyo3::wrap_pyfunction;
use pyo3_log;
use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};
use std::{fs::File, io::Write, path::PathBuf, sync::Arc};
use tokio::runtime::Runtime;

// See commands.rs and execute.rs
// RenderCircuit
// #[pyfunction]
// fn render_circuit(
//     data_path: &Path,
//     model: _,
//     output_path: &Path,
//     args: Vec<String>
// ) -> PyResult<()> {
//     let data = prepare_data(data_path.to_string());
//     let circuit = prepare_model_circuit::<Fr>(&data, &cli.args)?;

//     halo2_proofs::dev::CircuitLayout::default()
//         .show_labels(false)
//         .render(args.logrows, &circuit, &root)?;
// }

/// pyclass containing the struct used for run_args
#[pyclass]
#[derive(Clone)]
struct PyRunArgs {
    #[pyo3(get, set)]
    pub tolerance: Tolerance,
    #[pyo3(get, set)]
    pub scale: u32,
    #[pyo3(get, set)]
    pub bits: usize,
    #[pyo3(get, set)]
    pub logrows: u32,
    #[pyo3(get, set)]
    pub public_inputs: bool,
    #[pyo3(get, set)]
    pub public_outputs: bool,
    #[pyo3(get, set)]
    pub public_params: bool,
    #[pyo3(get, set)]
    pub pack_base: u32,
    #[pyo3(get, set)]
    pub allocated_constraints: Option<usize>,
}

/// default instantiation of PyRunArgs
#[pymethods]
impl PyRunArgs {
    #[new]
    fn new() -> Self {
        PyRunArgs {
            tolerance: Tolerance::Abs { val: 0 },
            scale: 7,
            bits: 16,
            logrows: 17,
            public_inputs: true,
            public_outputs: true,
            public_params: false,
            pack_base: 1,
            allocated_constraints: None,
        }
    }
}

/// Conversion between PyRunArgs and RunArgs
impl From<PyRunArgs> for RunArgs {
    fn from(py_run_args: PyRunArgs) -> Self {
        RunArgs {
            tolerance: py_run_args.tolerance,
            scale: py_run_args.scale,
            bits: py_run_args.bits,
            logrows: py_run_args.logrows,
            public_inputs: py_run_args.public_inputs,
            public_outputs: py_run_args.public_outputs,
            public_params: py_run_args.public_params,
            pack_base: py_run_args.pack_base,
            allocated_constraints: py_run_args.allocated_constraints,
        }
    }
}

/// Displays the table as a string in python
#[pyfunction(signature = (
    model,
    py_run_args = None
))]
fn table(model: String, py_run_args: Option<PyRunArgs>) -> Result<String, PyErr> {
    let run_args: RunArgs = py_run_args.unwrap_or_else(PyRunArgs::new).into();
    let visibility: VarVisibility = run_args.to_var_visibility();
    let mut reader = File::open(model).map_err(|_| PyIOError::new_err("Failed to open model"))?;
    let result = Model::<Fr>::new(&mut reader, run_args, Mode::Mock, visibility);

    match result {
        Ok(m) => Ok(m.table_nodes()),
        Err(_) => Err(PyIOError::new_err("Failed to import model")),
    }
}

/// generates the srs
#[pyfunction(signature = (
    params_path,
    logrows,
))]
fn gen_srs(params_path: PathBuf, logrows: usize) -> PyResult<()> {
    let params = ezkl_gen_srs::<KZGCommitmentScheme<Bn256>>(logrows as u32);
    save_params::<KZGCommitmentScheme<Bn256>>(&params_path, &params)?;
    Ok(())
}

/// runs the forward pass operation
#[pyfunction(signature = (
    data,
    model,
    output,
    py_run_args = None
))]
fn forward(
    data: String,
    model: String,
    output: String,
    py_run_args: Option<PyRunArgs>,
) -> PyResult<()> {
    let run_args: RunArgs = py_run_args.unwrap_or_else(PyRunArgs::new).into();
    let mut data = prepare_data(data).map_err(|_| PyIOError::new_err("Failed to import data"))?;

    let mut model_inputs = vec![];
    // quantize the supplied data using the provided scale.
    // for v in new_data.input_data.iter() {
    //     match vector_to_quantized(v, &Vec::from([v.len()]), 0.0, run_args.scale) {
    //         Ok(t) => model_inputs.push(t),
    //         Err(_) => return Err(PyValueError::new_err("Failed to quantize vector")),
    //     }
    // }
    for v in data.input_data.iter() {
        let t: Vec<i128> = v
            .par_iter()
            .map(|x| quantize_float(x, 0.0, run_args.scale).unwrap())
            .collect();
        model_inputs.push(t.into_iter().into());
    }
    let mut reader = File::open(model).map_err(|_| PyIOError::new_err("Failed to open model"))?;

    let model: Model<Fr> = Model::new(
        &mut reader,
        run_args,
        crate::graph::Mode::Prove,
        crate::graph::VarVisibility::default(),
    )
    .map_err(|_| PyIOError::new_err("Failed to create new model"))?;

    let res = model
        .forward(&model_inputs)
        .map_err(|_| PyIOError::new_err("Failed to run forward pass"))?;

    let output_scales = model.graph.get_output_scales();
    let output_scales = output_scales
        .iter()
        .map(|scale| crate::graph::scale_to_multiplier(*scale));

    let float_res: Vec<Vec<f32>> = res
        .iter()
        .zip(output_scales)
        .map(|(t, scale)| {
            t.iter()
                .map(|e| ((*e as f64) / scale) as f32)
                .collect::<Vec<f32>>()
        })
        .collect();
    trace!("forward pass output: {:?}", float_res);
    data.output_data = float_res;

    match serde_json::to_writer(&File::create(output)?, &data) {
        Ok(_) => {
            // TODO output a dictionary
            // obtain gil
            // TODO: Convert to Python::with_gil() when it stabilizes
            // let gil = Python::acquire_gil();
            // obtain python instance
            // let py = gil.python();
            // return Ok(new_data.to_object(py))
            Ok(())
        }
        Err(_) => return Err(PyIOError::new_err("Failed to create output file")),
    }
}

/// mocks the prover
#[pyfunction(signature = (
    data,
    model,
    py_run_args = None
))]
fn mock(data: String, model: String, py_run_args: Option<PyRunArgs>) -> Result<bool, PyErr> {
    let run_args: RunArgs = py_run_args.unwrap_or_else(PyRunArgs::new).into();
    let logrows = run_args.logrows;
    let data = prepare_data(data).map_err(|_| PyIOError::new_err("Failed to import data"))?;
    let visibility = run_args.to_var_visibility();
    let mut reader = File::open(model).map_err(|_| PyIOError::new_err("Failed to open model"))?;
    let procmodel = Model::<Fr>::new(&mut reader, run_args, Mode::Mock, visibility)
        .map_err(|_| PyIOError::new_err("Failed to process model"))?;

    let arcmodel: Arc<Model<Fr>> = Arc::new(procmodel);
    let circuit = ModelCircuit::<Fr>::new(&data, arcmodel, CheckMode::SAFE)
        .map_err(|_| PyRuntimeError::new_err("Failed to create circuit"))?;

    let public_inputs = circuit
        .prepare_public_inputs(&data)
        .map_err(|_| PyRuntimeError::new_err("Failed to prepare public inputs"))?;
    let prover = MockProver::run(logrows, &circuit, public_inputs)
        .map_err(|_| PyRuntimeError::new_err("Failed to run prover"))?;

    prover.assert_satisfied();

    let res = prover.verify();
    match res {
        Ok(_) => return Ok(true),
        Err(_) => return Ok(false),
    }
}

/// runs the prover on a set of inputs
#[pyfunction(signature = (
    data,
    model,
    vk_path,
    pk_path,
    params_path,
    circuit_params_path,
    py_run_args = None
))]
fn setup(
    data: String,
    model: String,
    vk_path: PathBuf,
    pk_path: PathBuf,
    params_path: PathBuf,
    circuit_params_path: PathBuf,
    py_run_args: Option<PyRunArgs>,
) -> Result<bool, PyErr> {
    let run_args: RunArgs = py_run_args.unwrap_or_else(PyRunArgs::new).into();
    let logrows = run_args.logrows;
    let data = prepare_data(data).map_err(|_| PyIOError::new_err("Failed to import data"))?;
    let visibility = run_args.to_var_visibility();

    let mut reader = File::open(model).map_err(|_| PyIOError::new_err("Failed to open model"))?;
    let procmodel = Model::<Fr>::new(&mut reader, run_args, Mode::Prove, visibility)
        .map_err(|_| PyIOError::new_err("Failed to process model"))?;

    let arcmodel: Arc<Model<Fr>> = Arc::new(procmodel);
    let circuit = ModelCircuit::<Fr>::new(&data, arcmodel, CheckMode::UNSAFE)
        .map_err(|_| PyRuntimeError::new_err("Failed to create circuit"))?;

    let params = load_params_cmd(params_path, logrows)
        .map_err(|_| PyIOError::new_err("Failed to load params"))?;

    let proving_key =
        create_keys::<KZGCommitmentScheme<Bn256>, Fr, ModelCircuit<Fr>>(&circuit, &params)
            .map_err(|_| PyRuntimeError::new_err("Failed to create proving key"))?;

    let circuit_params = circuit.params.clone();

    // save the verifier key
    save_vk::<KZGCommitmentScheme<Bn256>>(&vk_path, proving_key.get_vk())
        .map_err(|_| PyIOError::new_err("Failed to save verifier key to vk_path"))?;

    // save the prover key
    save_pk::<KZGCommitmentScheme<Bn256>>(&pk_path, &proving_key)
        .map_err(|_| PyIOError::new_err("Failed to save proving key to pk_path"))?;

    // save the circuit
    circuit_params.save(&circuit_params_path);

    Ok(true)
}

/// runs the prover on a set of inputs
#[pyfunction(signature = (
    data,
    model,
    pk_path,
    proof_path,
    params_path,
    transcript,
    strategy,
    circuit_params_path,
))]
fn prove(
    data: String,
    model: String,
    pk_path: PathBuf,
    proof_path: PathBuf,
    params_path: PathBuf,
    transcript: TranscriptType,
    strategy: StrategyType,
    circuit_params_path: PathBuf,
) -> Result<bool, PyErr> {
    let data = prepare_data(data).map_err(|_| PyIOError::new_err("Failed to import data"))?;

    let model_circuit_params = ModelParams::load(&circuit_params_path);

    let circuit = ModelCircuit::<Fr>::from_model_params(
        &data,
        &model_circuit_params,
        &model.into(),
        CheckMode::SAFE,
    )
    .map_err(|_| PyRuntimeError::new_err("Failed to create circuit"))?;

    let public_inputs = circuit
        .prepare_public_inputs(&data)
        .map_err(|_| PyRuntimeError::new_err("Failed to prepare public inputs"))?;

    let params = load_params_cmd(params_path, model_circuit_params.run_args.logrows)
        .map_err(|_| PyIOError::new_err("Failed to load params"))?;

    let proving_key = load_pk::<KZGCommitmentScheme<Bn256>, Fr, ModelCircuit<Fr>>(
        pk_path,
        circuit.params.clone(),
    )
    .map_err(|_| PyRuntimeError::new_err("Failed to create proving key"))?;

    let snark = match strategy {
        StrategyType::Single => {
            let strategy = KZGSingleStrategy::new(&params);
            match create_proof_circuit_kzg(
                circuit,
                &params,
                public_inputs,
                &proving_key,
                transcript,
                strategy,
                CheckMode::SAFE,
            ) {
                Ok(snark) => Ok(snark),
                Err(_) => Err(PyRuntimeError::new_err(
                    "Failed to create proof circuit single strategy",
                )),
            }
        }
        StrategyType::Accum => {
            let strategy = AccumulatorStrategy::new(&params);
            match create_proof_circuit_kzg(
                circuit,
                &params,
                public_inputs,
                &proving_key,
                transcript,
                strategy,
                CheckMode::SAFE,
            ) {
                Ok(snark) => Ok(snark),
                Err(_) => Err(PyRuntimeError::new_err(
                    "Failed to create proof circuit using accumulator strategy",
                )),
            }
        }
    };

    // save the snark proof
    snark?
        .save(&proof_path)
        .map_err(|_| PyIOError::new_err("Failed to save proof to proof path"))?;

    Ok(true)
}

/// verifies a given proof
#[pyfunction(signature = (
    proof_path,
    circuit_params_path,
    vk_path,
    params_path,
))]
fn verify(
    proof_path: PathBuf,
    circuit_params_path: PathBuf,
    vk_path: PathBuf,
    params_path: PathBuf,
) -> Result<bool, PyErr> {
    let model_circuit_params = ModelParams::load(&circuit_params_path);
    let params = load_params_cmd(params_path, model_circuit_params.run_args.logrows)
        .map_err(|_| PyIOError::new_err("Failed to load params"))?;
    let proof = Snark::load::<KZGCommitmentScheme<Bn256>>(&proof_path, None, None)
        .map_err(|_| PyIOError::new_err("Failed to load proof"))?;

    let strategy = KZGSingleStrategy::new(params.verifier_params());
    let vk =
        load_vk::<KZGCommitmentScheme<Bn256>, Fr, ModelCircuit<Fr>>(vk_path, model_circuit_params)
            .map_err(|_| PyIOError::new_err("Failed to load verifier key"))?;
    let result = verify_proof_circuit_kzg(params.verifier_params(), proof, &vk, strategy);
    match result {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// creates an EVM compatible verifier, you will need solc installed in your environment to run this
#[pyfunction(signature = (
    vk_path,
    params_path,
    circuit_params_path,
    deployment_code_path,
    sol_code_path=None,
))]
fn create_evm_verifier(
    vk_path: PathBuf,
    params_path: PathBuf,
    circuit_params_path: PathBuf,
    deployment_code_path: PathBuf,
    sol_code_path: Option<PathBuf>,
) -> Result<bool, PyErr> {
    let model_circuit_params = ModelParams::load(&circuit_params_path);
    let params = load_params_cmd(params_path, model_circuit_params.run_args.logrows)
        .map_err(|_| PyIOError::new_err("Failed to load model circuit parameters"))?;

    let num_instance = model_circuit_params
        .instance_shapes
        .iter()
        .map(|x| x.iter().product())
        .collect();

    let vk =
        load_vk::<KZGCommitmentScheme<Bn256>, Fr, ModelCircuit<Fr>>(vk_path, model_circuit_params)
            .map_err(|_| PyIOError::new_err("Failed to load verifier key"))?;

    let (deployment_code, yul_code) = gen_evm_verifier(&params, &vk, num_instance)
        .map_err(|_| PyRuntimeError::new_err("Failed to generatee evm verifier"))?;

    deployment_code
        .save(&deployment_code_path)
        .map_err(|_| PyIOError::new_err("Failed to save deployment code"))?;

    if sol_code_path.is_some() {
        let mut f = File::create(sol_code_path.as_ref().unwrap())
            .map_err(|_| PyIOError::new_err("Failed to create file"))?;
        let _ = f.write(yul_code.as_bytes());

        let output = fix_verifier_sol(sol_code_path.as_ref().unwrap().clone())
            .map_err(|_| PyRuntimeError::new_err("Failed to fix solidity verifier"))?;

        let mut f = File::create(sol_code_path.as_ref().unwrap())
            .map_err(|_| PyIOError::new_err("Failed to write solidity code into file"))?;
        let _ = f.write(output.as_bytes());
    }
    Ok(true)
}

// verifies an evm compatible proof, you will need solc installed in your environment to run this
#[pyfunction(signature = (
    proof_path,
    deployment_code_path,
    sol_code_path=None,
))]
fn verify_evm(
    proof_path: PathBuf,
    deployment_code_path: PathBuf,
    sol_code_path: Option<PathBuf>,
) -> Result<bool, PyErr> {
    let proof = Snark::load::<KZGCommitmentScheme<Bn256>>(&proof_path, None, None)
        .map_err(|_| PyIOError::new_err("Failed to load proof"))?;
    let code = DeploymentCode::load(&deployment_code_path)
        .map_err(|_| PyIOError::new_err("Failed to load deployment code path"))?;
    evm_verify(code, proof.clone())
        .map_err(|_| PyRuntimeError::new_err("Failed to verify with evm"))?;

    if sol_code_path.is_some() {
        let result = Runtime::new()
            .unwrap()
            .block_on(verify_proof_via_solidity(proof, sol_code_path.unwrap()))
            .map_err(|_| PyRuntimeError::new_err("Failed to verify proof via solidity"))?;

        trace!("Solidity verification result: {}", result);

        assert!(result);
    }
    Ok(true)
}

// TODO: CreateEVMVerifierAggr
// TODO: DeployVerifierEVM (To be done in pyezkl)
// TODO: Aggregate
// TODO: SendProofEVM
// TODO: VerifyAggr
// TODO: PrintProofHex

// Python Module
#[pymodule]
fn ezkl_lib(_py: Python<'_>, m: &PyModule) -> PyResult<()> {
    pyo3_log::init();
    m.add_class::<PyRunArgs>()?;
    m.add_function(wrap_pyfunction!(table, m)?)?;
    m.add_function(wrap_pyfunction!(gen_srs, m)?)?;
    m.add_function(wrap_pyfunction!(forward, m)?)?;
    m.add_function(wrap_pyfunction!(mock, m)?)?;
    m.add_function(wrap_pyfunction!(setup, m)?)?;
    m.add_function(wrap_pyfunction!(prove, m)?)?;
    m.add_function(wrap_pyfunction!(verify, m)?)?;
    m.add_function(wrap_pyfunction!(create_evm_verifier, m)?)?;
    m.add_function(wrap_pyfunction!(verify_evm, m)?)?;

    Ok(())
}
