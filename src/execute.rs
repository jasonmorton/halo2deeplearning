use crate::circuit::CheckMode;
#[cfg(not(target_arch = "wasm32"))]
use crate::commands::{CalibrationTarget, StrategyType};
use crate::commands::{Cli, Commands};
#[cfg(not(target_arch = "wasm32"))]
use crate::eth::{deploy_da_verifier_via_solidity, deploy_verifier_via_solidity};
#[cfg(not(target_arch = "wasm32"))]
use crate::eth::{fix_verifier_sol, get_contract_artifacts, verify_proof_via_solidity};
use crate::graph::input::GraphData;
use crate::graph::{GraphCircuit, GraphSettings, GraphWitness, Model};
#[cfg(not(target_arch = "wasm32"))]
use crate::graph::{TestDataSource, TestSources};
use crate::pfsys::evm::aggregation::AggregationCircuit;
#[cfg(not(target_arch = "wasm32"))]
use crate::pfsys::evm::evm_verify;
#[cfg(not(target_arch = "wasm32"))]
use crate::pfsys::evm::{
    aggregation::gen_aggregation_evm_verifier, single::gen_evm_verifier, DeploymentCode, YulCode,
};
use crate::pfsys::{create_keys, load_pk, load_vk, save_params, save_pk, Snark, TranscriptType};
use crate::pfsys::{create_proof_circuit_kzg, verify_proof_circuit_kzg};
use crate::pfsys::{save_vk, srs::*};
use crate::RunArgs;
#[cfg(not(target_arch = "wasm32"))]
use ethers::types::H160;
#[cfg(not(target_arch = "wasm32"))]
use gag::Gag;
use halo2_proofs::dev::VerifyFailure;
use halo2_proofs::poly::commitment::Params;
use halo2_proofs::poly::commitment::ParamsProver;
use halo2_proofs::poly::kzg::commitment::KZGCommitmentScheme;
use halo2_proofs::poly::kzg::strategy::AccumulatorStrategy;
use halo2_proofs::poly::kzg::{
    commitment::ParamsKZG, strategy::SingleStrategy as KZGSingleStrategy,
};
use halo2curves::bn256::{Bn256, Fr, G1Affine};
#[cfg(not(target_arch = "wasm32"))]
use halo2curves::ff::Field;
#[cfg(not(target_arch = "wasm32"))]
use indicatif::{ProgressBar, ProgressStyle};
use instant::Instant;
#[cfg(not(target_arch = "wasm32"))]
use itertools::Itertools;
#[cfg(not(target_arch = "wasm32"))]
use log::debug;
use log::{info, trace};
#[cfg(feature = "render")]
use plotters::prelude::*;
#[cfg(not(target_arch = "wasm32"))]
use rand::Rng;
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
#[cfg(not(target_arch = "wasm32"))]
use snark_verifier::loader::evm;
use std::error::Error;
use std::fs::File;
#[cfg(not(target_arch = "wasm32"))]
use std::io::ErrorKind::NotFound;
#[cfg(not(target_arch = "wasm32"))]
use std::io::{Cursor, Write};
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::process::Command;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::OnceLock;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;
use thiserror::Error;

#[cfg(not(target_arch = "wasm32"))]
static _SOLC_REQUIREMENT: OnceLock<bool> = OnceLock::new();
#[cfg(not(target_arch = "wasm32"))]
fn check_solc_requirement() {
    info!("checking solc installation..");
    _SOLC_REQUIREMENT.get_or_init(|| match Command::new("solc").arg("--version").output() {
        Ok(output) => {
            #[cfg(not(target_arch = "wasm32"))]
            debug!("solc output: {:#?}", output);
            #[cfg(not(target_arch = "wasm32"))]
            debug!("solc output success: {:#?}", output.status.success());
            assert!(
                output.status.success(),
                "`solc` check failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            #[cfg(not(target_arch = "wasm32"))]
            debug!("solc check passed, proceeding");
            true
        }
        Err(e) => {
            if let NotFound = e.kind() {
                panic!(
                    "`solc` was not found! Consider using solc-select or check your PATH! {}",
                    e
                );
            } else {
                panic!("`solc` check failed: {}", e);
            }
        }
    });
}

/// A wrapper for tensor related errors.
#[derive(Debug, Error)]
pub enum ExecutionError {
    /// Shape mismatch in a operation
    #[error("verification failed")]
    VerifyError(Vec<VerifyFailure>),
}

/// Run an ezkl command with given args
pub async fn run(cli: Cli) -> Result<(), Box<dyn Error>> {
    match cli.command {
        #[cfg(not(target_arch = "wasm32"))]
        Commands::Fuzz {
            witness,
            compiled_model,
            transcript,
            args,
            num_runs,
            settings_path,
        } => {
            fuzz(
                compiled_model,
                args.logrows,
                witness,
                transcript,
                num_runs,
                args,
                settings_path,
            )
            .await
        }

        Commands::GenSrs { srs_path, logrows } => gen_srs_cmd(srs_path, logrows as u32),
        #[cfg(not(target_arch = "wasm32"))]
        Commands::GetSrs {
            srs_path,
            settings_path,
            logrows,
            check,
        } => get_srs_cmd(srs_path, settings_path, logrows, check).await,
        Commands::Table { model, args } => table(model, args),
        #[cfg(feature = "render")]
        Commands::RenderCircuit {
            model,
            output,
            args,
        } => render(model, output, args),
        Commands::GenSettings {
            model,
            settings_path,
            args,
        } => gen_circuit_settings(model, settings_path, args),
        #[cfg(not(target_arch = "wasm32"))]
        Commands::CalibrateSettings {
            model,
            settings_path,
            data,
            target,
        } => calibrate(model, data, settings_path, target).await,
        Commands::GenWitness {
            data,
            compiled_model,
            output,
            settings_path,
        } => gen_witness(compiled_model, data, Some(output), settings_path)
            .await
            .map(|_| ()),
        Commands::Mock {
            model,
            witness,
            settings_path,
        } => mock(model, witness, settings_path).await,
        #[cfg(not(target_arch = "wasm32"))]
        Commands::CreateEVMVerifier {
            vk_path,
            srs_path,
            settings_path,
            sol_code_path,
            abi_path,
        } => create_evm_verifier(vk_path, srs_path, settings_path, sol_code_path, abi_path),
        #[cfg(not(target_arch = "wasm32"))]
        Commands::CreateEVMDataAttestationVerifier {
            vk_path,
            srs_path,
            settings_path,
            sol_code_path,
            abi_path,
            data,
        } => create_evm_data_attestation_verifier(
            vk_path,
            srs_path,
            settings_path,
            sol_code_path,
            abi_path,
            data,
        ),
        #[cfg(not(target_arch = "wasm32"))]
        Commands::CreateEVMVerifierAggr {
            vk_path,
            srs_path,
            sol_code_path,
            abi_path,
            aggregation_settings,
        } => create_evm_aggregate_verifier(
            vk_path,
            srs_path,
            sol_code_path,
            abi_path,
            aggregation_settings,
        ),
        Commands::CompileModel {
            model,
            compiled_model,
            settings_path,
        } => compile_model(model, compiled_model, settings_path),
        Commands::Setup {
            compiled_model,
            srs_path,
            settings_path,
            vk_path,
            pk_path,
        } => setup(compiled_model, srs_path, settings_path, vk_path, pk_path),
        #[cfg(not(target_arch = "wasm32"))]
        Commands::SetupTestEVMData {
            data,
            compiled_model,
            settings_path,
            test_data,
            rpc_url,
            input_source,
            output_source,
        } => {
            setup_test_evm_witness(
                data,
                compiled_model,
                settings_path,
                test_data,
                rpc_url,
                input_source,
                output_source,
            )
            .await
        }
        #[cfg(not(target_arch = "wasm32"))]
        Commands::Prove {
            witness,
            compiled_model,
            pk_path,
            proof_path,
            srs_path,
            transcript,
            strategy,
            settings_path,
            check_mode,
        } => prove(
            witness,
            compiled_model,
            pk_path,
            Some(proof_path),
            srs_path,
            transcript,
            strategy,
            settings_path,
            check_mode,
        )
        .await
        .map(|_| ()),
        Commands::MockAggregate {
            aggregation_snarks,
            logrows,
        } => mock_aggregate(aggregation_snarks, logrows),
        Commands::SetupAggregate {
            sample_snarks,
            vk_path,
            pk_path,
            srs_path,
            logrows,
        } => setup_aggregate(sample_snarks, vk_path, pk_path, srs_path, logrows),
        Commands::Aggregate {
            proof_path,
            aggregation_snarks,
            pk_path,
            srs_path,
            transcript,
            logrows,
            check_mode,
        } => aggregate(
            proof_path,
            aggregation_snarks,
            pk_path,
            srs_path,
            transcript,
            logrows,
            check_mode,
        ),
        Commands::Verify {
            proof_path,
            settings_path,
            vk_path,
            srs_path,
        } => verify(proof_path, settings_path, vk_path, srs_path),
        Commands::VerifyAggr {
            proof_path,
            vk_path,
            srs_path,
            logrows,
        } => verify_aggr(proof_path, vk_path, srs_path, logrows),
        #[cfg(not(target_arch = "wasm32"))]
        Commands::DeployEvmVerifier {
            sol_code_path,
            rpc_url,
            addr_path,
            optimizer_runs,
        } => deploy_evm(sol_code_path, rpc_url, addr_path, optimizer_runs).await,
        #[cfg(not(target_arch = "wasm32"))]
        Commands::DeployEvmDataAttestationVerifier {
            data,
            settings_path,
            sol_code_path,
            rpc_url,
            addr_path,
            optimizer_runs,
        } => {
            deploy_da_evm(
                data,
                settings_path,
                sol_code_path,
                rpc_url,
                addr_path,
                optimizer_runs,
            )
            .await
        }
        #[cfg(not(target_arch = "wasm32"))]
        Commands::VerifyEVM {
            proof_path,
            addr,
            rpc_url,
            data_attestation,
        } => verify_evm(proof_path, addr, rpc_url, data_attestation).await,
        Commands::PrintProofHex { proof_path } => print_proof_hex(proof_path),
    }
}

pub(crate) fn gen_srs_cmd(srs_path: PathBuf, logrows: u32) -> Result<(), Box<dyn Error>> {
    let params = gen_srs::<KZGCommitmentScheme<Bn256>>(logrows);
    save_params::<KZGCommitmentScheme<Bn256>>(&srs_path, &params)?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
async fn fetch_srs(uri: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let pb = {
        let pb = init_spinner();
        pb.set_message("Downloading SRS (this may take a while) ...");
        pb
    };
    let client = reqwest::Client::new();
    // wasm doesn't require it to be mutable
    #[allow(unused_mut)]
    let mut resp = client.get(uri).body(vec![]).send().await?;
    let mut buf = vec![];
    while let Some(chunk) = resp.chunk().await? {
        buf.extend(chunk.to_vec());
    }

    pb.finish_with_message("SRS downloaded.");
    Ok(buf.drain(..buf.len()).collect())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn get_srs_cmd(
    srs_path: PathBuf,
    settings_path: Option<PathBuf>,
    logrows: Option<u32>,
    check_mode: CheckMode,
) -> Result<(), Box<dyn Error>> {
    let k = if let Some(settings_p) = settings_path {
        if settings_p.exists() {
            let settings = GraphSettings::load(&settings_p)?;
            settings.run_args.logrows
        } else {
            let err_string = format!(
                "You will need to provide a valid settings file to use the settings option. You should run gen-settings to generate a settings file (and calibrate-settings to pick optimal logrows)."
            );
            return Err(err_string.into());
        }
    } else if let Some(k) = logrows {
        k
    } else {
        let err_string = format!(
            "You will need to provide a settings file or set the logrows. You should run gen-settings to generate a settings file (and calibrate-settings to pick optimal logrows)."
        );
        return Err(err_string.into());
    };

    let srs_uri = format!("{}{}", PUBLIC_SRS_URL, k);
    let mut reader = Cursor::new(fetch_srs(&srs_uri).await?);
    // check the SRS
    if matches!(check_mode, CheckMode::SAFE) {
        #[cfg(not(target_arch = "wasm32"))]
        let pb = init_spinner();
        #[cfg(not(target_arch = "wasm32"))]
        pb.set_message("Validating SRS (this may take a while) ...");
        ParamsKZG::<Bn256>::read(&mut reader)?;
        #[cfg(not(target_arch = "wasm32"))]
        pb.finish_with_message("SRS validated");
    }

    let mut file = std::fs::File::create(srs_path)?;
    file.write_all(reader.get_ref())?;

    info!("SRS downloaded");
    Ok(())
}

pub(crate) fn table(model: PathBuf, run_args: RunArgs) -> Result<(), Box<dyn Error>> {
    let model = Model::from_run_args(&run_args, &model)?;
    info!("\n {}", model.table_nodes());
    Ok(())
}

pub(crate) async fn gen_witness(
    compiled_model_path: PathBuf,
    data: PathBuf,
    output: Option<PathBuf>,
    settings_path: PathBuf,
) -> Result<GraphWitness, Box<dyn Error>> {
    // these aren't real values so the sanity checks are mostly meaningless

    let circuit_settings = GraphSettings::load(&settings_path)?;

    let mut circuit = GraphCircuit::preprocessed_from_settings(
        &circuit_settings,
        &compiled_model_path,
        CheckMode::UNSAFE,
    )?;
    let data = GraphData::from_path(data)?;

    #[cfg(not(target_arch = "wasm32"))]
    let input = circuit.load_graph_input(&data).await?;
    #[cfg(target_arch = "wasm32")]
    let input = circuit.load_graph_input(&data)?;

    let start_time = Instant::now();

    let witness = circuit.forward(&input)?;

    trace!(
        "witness generation (B={:?}) took {:?}",
        circuit_settings.run_args.batch_size,
        start_time.elapsed()
    );

    if let Some(output_path) = output {
        serde_json::to_writer(&File::create(output_path)?, &witness)?;
    }
    Ok(witness)
}

/// Generate a circuit settings file
pub(crate) fn gen_circuit_settings(
    model_path: PathBuf,
    params_output: PathBuf,
    run_args: RunArgs,
) -> Result<(), Box<dyn Error>> {
    let circuit = GraphCircuit::from_run_args(&run_args, &model_path)?;
    let params = circuit.settings;
    params.save(&params_output).map_err(Box::<dyn Error>::from)
}

// not for wasm targets
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn init_spinner() -> ProgressBar {
    let pb = indicatif::ProgressBar::new_spinner();
    pb.set_draw_target(indicatif::ProgressDrawTarget::stdout());
    pb.enable_steady_tick(Duration::from_millis(200));
    pb.set_style(
        ProgressStyle::with_template("[{elapsed_precise}] {spinner:.blue} {msg}")
            .unwrap()
            .tick_strings(&[
                "------ - ✨ ",
                "------ - ⏳ ",
                "------ - 🌎 ",
                "------ - 🔎 ",
                "------ - 🥹 ",
                "------ - 🫠 ",
                "------ - 👾 ",
            ]),
    );
    pb
}

// not for wasm targets
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn init_bar(len: u64) -> ProgressBar {
    let pb = indicatif::ProgressBar::new(len);
    pb.set_draw_target(indicatif::ProgressDrawTarget::stdout());
    pb.enable_steady_tick(Duration::from_millis(200));
    let sty = ProgressStyle::with_template(
        "[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}",
    )
    .unwrap()
    .progress_chars("##-");
    pb.set_style(sty);

    pb
}

/// Calibrate the circuit parameters to a given a dataset
#[cfg(not(target_arch = "wasm32"))]
#[allow(trivial_casts)]
pub(crate) async fn calibrate(
    model_path: PathBuf,
    data: PathBuf,
    settings_path: PathBuf,
    target: CalibrationTarget,
) -> Result<(), Box<dyn Error>> {
    let data = GraphData::from_path(data)?;
    // load the pre-generated settings
    let settings = GraphSettings::load(&settings_path)?;
    // now retrieve the run args
    let run_args = settings.run_args;

    let pb = init_bar((2..16).len() as u64);

    pb.set_message("calibrating...");
    // we load the model to get the input and output shapes
    let _r = Gag::stdout().unwrap();
    let model = Model::from_run_args(&run_args, &model_path).unwrap();
    std::mem::drop(_r);

    let chunks = data
        .split_into_batches(run_args.batch_size, model.graph.input_shapes())
        .unwrap();

    debug!("num of calibration batches: {}", chunks.len(),);

    let mut found_params: Vec<GraphSettings> = vec![];

    for scale in 2..16 {
        pb.set_message(format!("scale {}", scale));
        std::thread::sleep(Duration::from_millis(100));

        // let _r = Gag::stdout().unwrap();
        // Result<Vec<GraphSettings>, &str>
        let tasks = chunks
            .iter()
            .map(|chunk| {
                // we need to create a new run args for each chunk
                // time it
                let chunk = chunk.clone();
                let local_run_args = RunArgs { scale, ..run_args };

                let original_settings = settings.clone();

                let mut circuit = GraphCircuit::from_run_args(&local_run_args, &model_path)
                    .map_err(|_| "failed to create circuit from run args")
                    .unwrap();

                tokio::task::spawn(async move {
                    let data = circuit
                        .load_graph_input(&chunk)
                        .await
                        .map_err(|_| "failed to load circuit inputs")
                        .unwrap();

                    loop {
                        // ensures we have converged
                        let params_before = circuit.settings.clone();
                        circuit
                            .calibrate(&data)
                            .map_err(|_| "failed to calibrate")?;
                        let params_after = circuit.settings.clone();
                        if params_before == params_after {
                            break;
                        }
                    }

                    let found_run_args = RunArgs {
                        scale: circuit.settings.run_args.scale,
                        bits: circuit.settings.run_args.bits,
                        logrows: circuit.settings.run_args.logrows,
                        ..run_args
                    };

                    let found_settings = GraphSettings {
                        run_args: found_run_args,
                        required_lookups: circuit.settings.required_lookups,
                        model_output_scales: circuit.settings.model_output_scales,
                        num_constraints: circuit.settings.num_constraints,
                        ..original_settings.clone()
                    };

                    Ok(found_settings) as Result<GraphSettings, &str>
                })
            })
            .collect::<Vec<tokio::task::JoinHandle<std::result::Result<GraphSettings, &str>>>>();

        let mut res: Vec<GraphSettings> = vec![];
        for task in tasks {
            if let Ok(task) = task.await? {
                res.push(task);
            }
        }
        if let Some(best) = res
            .into_iter()
            .max_by_key(|p| (p.run_args.bits, p.run_args.scale))
        {
            // pick the one with the largest logrows
            found_params.push(best);
        }

        // std::mem::drop(_r);
        pb.inc(1);
    }

    pb.finish_with_message("Calibration Done.");

    if found_params.is_empty() {
        return Err("calibration failed, could not find any suitable parameters given the calibration dataset".into());
    }

    debug!("Found {} sets of parameters", found_params.len());

    // now find the best params according to the target
    let best_params = match target {
        CalibrationTarget::Resources => {
            let mut param_iterator = found_params.iter().sorted_by_key(|p| p.run_args.logrows);

            let min_logrows = param_iterator.next().unwrap().run_args.logrows;

            // pick the ones that have the minimum logrows but also the largest scale:
            // this is the best tradeoff between resource usage and accuracy
            found_params
                .iter()
                .filter(|p| p.run_args.logrows == min_logrows)
                .max_by_key(|p| p.run_args.scale)
                .unwrap()
                .clone()
        }
        CalibrationTarget::Accuracy => {
            let param_iterator = found_params.iter().sorted_by_key(|p| p.run_args.scale);

            let max_scale = param_iterator.last().unwrap().run_args.scale;

            // pick the ones that have the max scale but also the smallest logrows:
            // this is the best tradeoff between resource usage and accuracy
            found_params
                .iter()
                .filter(|p| p.run_args.scale == max_scale)
                .min_by_key(|p| p.run_args.logrows)
                .unwrap()
                .clone()
        }
    };

    best_params.save(&settings_path)?;

    debug!("Saved parameters.");

    Ok(())
}

pub(crate) async fn mock(
    compiled_model_path: PathBuf,
    data_path: PathBuf,
    settings_path: PathBuf,
) -> Result<(), Box<dyn Error>> {
    // mock should catch any issues by default so we set it to safe
    let circuit_settings = GraphSettings::load(&settings_path)?;
    let mut circuit = GraphCircuit::preprocessed_from_settings(
        &circuit_settings,
        &compiled_model_path,
        CheckMode::SAFE,
    )?;

    let data = GraphWitness::from_path(data_path)?;

    circuit.load_graph_witness(&data)?;

    let public_inputs = circuit.prepare_public_inputs(&data)?;

    info!("Mock proof");

    let prover = halo2_proofs::dev::MockProver::run(
        circuit.settings.run_args.logrows,
        &circuit,
        public_inputs,
    )
    .map_err(Box::<dyn Error>::from)?;
    prover
        .verify_par()
        .map_err(|e| Box::<dyn Error>::from(ExecutionError::VerifyError(e)))?;
    Ok(())
}

pub(crate) fn print_proof_hex(proof_path: PathBuf) -> Result<(), Box<dyn Error>> {
    let proof = Snark::load::<KZGCommitmentScheme<Bn256>>(&proof_path)?;
    for instance in proof.instances {
        println!("{:?}", instance);
    }
    info!("{}", hex::encode(proof.proof));
    Ok(())
}
/// helper function to generate the deployment code from yul code
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn gen_deployment_code(yul_code: YulCode) -> Result<DeploymentCode, Box<dyn Error>> {
    Ok(DeploymentCode {
        code: evm::compile_yul(&yul_code),
    })
}

#[cfg(feature = "render")]
pub(crate) fn render(model: PathBuf, output: PathBuf, args: RunArgs) -> Result<(), Box<dyn Error>> {
    let circuit = GraphCircuit::from_run_args(&args, &model)?;
    info!("Rendering circuit");

    // Create the area we want to draw on.
    // We could use SVGBackend if we want to render to .svg instead.
    // for an overview of how to interpret these plots, see https://zcash.github.io/halo2/user/dev-tools.html
    let root = BitMapBackend::new(&output, (512, 512)).into_drawing_area();
    root.fill(&TRANSPARENT).unwrap();
    let root = root.titled("Layout", ("sans-serif", 20))?;

    halo2_proofs::dev::CircuitLayout::default()
        // We hide labels, else most circuits become impossible to decipher because of overlaid text
        .show_labels(false)
        .render(circuit.settings.run_args.logrows, &circuit, &root)?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn create_evm_verifier(
    vk_path: PathBuf,
    srs_path: PathBuf,
    settings_path: PathBuf,
    sol_code_path: PathBuf,
    abi_path: PathBuf,
) -> Result<(), Box<dyn Error>> {
    check_solc_requirement();
    let circuit_settings = GraphSettings::load(&settings_path)?;
    let params = load_params_cmd(srs_path, circuit_settings.run_args.logrows)?;

    let num_instance = circuit_settings.total_instances();

    let vk = load_vk::<KZGCommitmentScheme<Bn256>, Fr, GraphCircuit>(vk_path, circuit_settings)?;
    trace!("params computed");

    let yul_code: YulCode = gen_evm_verifier(&params, &vk, num_instance.clone())?;

    let mut f = File::create(sol_code_path.clone())?;
    let _ = f.write(yul_code.as_bytes());

    let output = fix_verifier_sol(
        sol_code_path.clone(),
        num_instance.iter().sum::<usize>().try_into().unwrap(),
        None,
        None,
    )?;

    let mut f = File::create(sol_code_path.clone())?;
    let _ = f.write(output.as_bytes());

    // fetch abi of the contract
    let (abi, _, _) = get_contract_artifacts(sol_code_path, "Verifier", None)?;
    // save abi to file
    serde_json::to_writer(std::fs::File::create(abi_path)?, &abi)?;

    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn create_evm_data_attestation_verifier(
    vk_path: PathBuf,
    srs_path: PathBuf,
    settings_path: PathBuf,
    sol_code_path: PathBuf,
    abi_path: PathBuf,
    input: PathBuf,
) -> Result<(), Box<dyn Error>> {
    use crate::graph::{DataSource, VarVisibility};
    check_solc_requirement();

    let settings = GraphSettings::load(&settings_path)?;
    let params = load_params_cmd(srs_path, settings.run_args.logrows)?;

    let visibility = VarVisibility::from_args(settings.run_args)?;

    let num_instance = settings.total_instances();

    let vk = load_vk::<KZGCommitmentScheme<Bn256>, Fr, GraphCircuit>(vk_path, settings.clone())?;
    trace!("params computed");

    let yul_code: YulCode = gen_evm_verifier(&params, &vk, num_instance.clone())?;

    let mut f = File::create(sol_code_path.clone())?;
    let _ = f.write(yul_code.as_bytes());

    let data = GraphData::from_path(input)?;

    let output_data = if let Some(DataSource::OnChain(source)) = data.output_data {
        if !visibility.output.is_public() {
            todo!("we currently don't support private output data on chain")
        }
        let mut on_chain_output_data = vec![];
        for call in source.calls {
            on_chain_output_data.push(call);
        }
        Some(on_chain_output_data)
    } else {
        None
    };

    let input_data = if let DataSource::OnChain(source) = data.input_data {
        if !visibility.input.is_public() {
            todo!("we currently don't support private input data on chain")
        }
        let mut on_chain_input_data = vec![];
        for call in source.calls {
            on_chain_input_data.push(call);
        }
        Some((settings.run_args.scale, on_chain_input_data))
    } else {
        None
    };

    if input_data.is_some() || output_data.is_some() {
        let output = fix_verifier_sol(
            sol_code_path.clone(),
            num_instance.iter().sum::<usize>().try_into().unwrap(),
            input_data,
            output_data,
        )?;
        let mut f = File::create(sol_code_path.clone())?;
        let _ = f.write(output.as_bytes());
        // fetch abi of the contract
        let (abi, _, _) = get_contract_artifacts(sol_code_path, "DataAttestationVerifier", None)?;
        // save abi to file
        serde_json::to_writer(std::fs::File::create(abi_path)?, &abi)?;
    } else {
        return Err(
            "Neither input or output data source is on-chain. Atleast one must be on chain.".into(),
        );
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn deploy_da_evm(
    data: PathBuf,
    settings_path: PathBuf,
    sol_code_path: PathBuf,
    rpc_url: Option<String>,
    addr_path: PathBuf,
    runs: Option<usize>,
) -> Result<(), Box<dyn Error>> {
    check_solc_requirement();
    let contract_address = deploy_da_verifier_via_solidity(
        settings_path,
        data,
        sol_code_path,
        rpc_url.as_deref(),
        runs,
    )
    .await?;
    info!("Contract deployed at: {}", contract_address);

    let mut f = File::create(addr_path)?;
    write!(f, "{:#?}", contract_address)?;

    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn deploy_evm(
    sol_code_path: PathBuf,
    rpc_url: Option<String>,
    addr_path: PathBuf,
    runs: Option<usize>,
) -> Result<(), Box<dyn Error>> {
    check_solc_requirement();
    let contract_address =
        deploy_verifier_via_solidity(sol_code_path, rpc_url.as_deref(), runs).await?;

    info!("Contract deployed at: {:#?}", contract_address);

    let mut f = File::create(addr_path)?;
    write!(f, "{:#?}", contract_address)?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn verify_evm(
    proof_path: PathBuf,
    addr: H160,
    rpc_url: Option<String>,
    uses_data_attestation: bool,
) -> Result<(), Box<dyn Error>> {
    use crate::eth::verify_proof_with_data_attestation;
    check_solc_requirement();

    let proof = Snark::load::<KZGCommitmentScheme<Bn256>>(&proof_path)?;

    let result = if !uses_data_attestation {
        verify_proof_via_solidity(proof.clone(), addr, rpc_url.as_deref()).await?
    } else {
        verify_proof_with_data_attestation(proof.clone(), addr, rpc_url.as_deref()).await?
    };

    info!("Solidity verification result: {}", result);

    assert!(result);

    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn create_evm_aggregate_verifier(
    vk_path: PathBuf,
    srs_path: PathBuf,
    sol_code_path: PathBuf,
    abi_path: PathBuf,
    circuit_settings: Vec<PathBuf>,
) -> Result<(), Box<dyn Error>> {
    check_solc_requirement();
    let params: ParamsKZG<Bn256> = load_srs::<KZGCommitmentScheme<Bn256>>(srs_path)?;

    let settings: Vec<GraphSettings> = circuit_settings
        .iter()
        .map(|path| GraphSettings::load(path).unwrap())
        .collect::<Vec<_>>();

    let num_public_inputs: usize = settings
        .iter()
        .map(|s| s.total_instances().iter().sum::<usize>())
        .sum();

    let agg_vk = load_vk::<KZGCommitmentScheme<Bn256>, Fr, AggregationCircuit>(vk_path, ())?;

    let yul_code = gen_aggregation_evm_verifier(
        &params,
        &agg_vk,
        AggregationCircuit::num_instance(num_public_inputs),
        AggregationCircuit::accumulator_indices(),
    )?;

    let mut f = File::create(sol_code_path.clone())?;
    let _ = f.write(yul_code.as_bytes());

    let output = fix_verifier_sol(
        sol_code_path.clone(),
        AggregationCircuit::num_instance(num_public_inputs)
            .iter()
            .sum::<usize>()
            .try_into()
            .unwrap(),
        None,
        None,
    )?;

    let mut f = File::create(sol_code_path.clone())?;
    let _ = f.write(output.as_bytes());

    // fetch abi of the contract
    let (abi, _, _) = get_contract_artifacts(sol_code_path, "Verifier", None)?;
    // save abi to file
    serde_json::to_writer(std::fs::File::create(abi_path)?, &abi)?;

    Ok(())
}

pub(crate) fn compile_model(
    model_path: PathBuf,
    compiled_model: PathBuf,
    settings_path: PathBuf,
) -> Result<(), Box<dyn Error>> {
    let settings = GraphSettings::load(&settings_path)?;
    let model = Model::from_run_args(&settings.run_args, &model_path)?;
    model.save(compiled_model)?;
    Ok(())
}

pub(crate) fn setup(
    compiled_model: PathBuf,
    srs_path: PathBuf,
    settings_path: PathBuf,
    vk_path: PathBuf,
    pk_path: PathBuf,
) -> Result<(), Box<dyn Error>> {
    // these aren't real values so the sanity checks are mostly meaningless
    let circuit_settings = GraphSettings::load(&settings_path)?;
    let circuit = GraphCircuit::preprocessed_from_settings(
        &circuit_settings,
        &compiled_model,
        CheckMode::UNSAFE,
    )?;
    let params = load_params_cmd(srs_path, circuit_settings.run_args.logrows)?;

    let pk = create_keys::<KZGCommitmentScheme<Bn256>, Fr, GraphCircuit>(&circuit, &params)
        .map_err(Box::<dyn Error>::from)?;

    save_vk::<KZGCommitmentScheme<Bn256>>(&vk_path, pk.get_vk())?;
    save_pk::<KZGCommitmentScheme<Bn256>>(&pk_path, &pk)?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn setup_test_evm_witness(
    data_path: PathBuf,
    compiled_model_path: PathBuf,
    settings_path: PathBuf,
    test_data: PathBuf,
    rpc_url: Option<String>,
    input_source: TestDataSource,
    output_source: TestDataSource,
) -> Result<(), Box<dyn Error>> {
    use crate::graph::TestOnChainData;

    info!("run this command in background to keep the instance running for testing");
    let mut data = GraphData::from_path(data_path)?;
    let circuit_settings = GraphSettings::load(&settings_path)?;
    let mut circuit = GraphCircuit::preprocessed_from_settings(
        &circuit_settings,
        &compiled_model_path,
        CheckMode::SAFE,
    )?;

    // if both input and output are from files fail
    if matches!(input_source, TestDataSource::File) && matches!(output_source, TestDataSource::File)
    {
        return Err("Both input and output cannot be from files".into());
    }

    let test_on_chain_data = TestOnChainData {
        data: test_data.clone(),
        rpc: rpc_url,
        data_sources: TestSources {
            input: input_source,
            output: output_source,
        },
    };

    circuit
        .populate_on_chain_test_data(&mut data, test_on_chain_data)
        .await?;

    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn prove(
    data_path: PathBuf,
    compiled_model_path: PathBuf,
    pk_path: PathBuf,
    proof_path: Option<PathBuf>,
    srs_path: PathBuf,
    transcript: TranscriptType,
    strategy: StrategyType,
    settings_path: PathBuf,
    check_mode: CheckMode,
) -> Result<Snark<Fr, G1Affine>, Box<dyn Error>> {
    let data = GraphWitness::from_path(data_path)?;
    let circuit_settings = GraphSettings::load(&settings_path)?;
    let mut circuit = GraphCircuit::preprocessed_from_settings(
        &circuit_settings,
        &compiled_model_path,
        check_mode,
    )?;

    circuit.load_graph_witness(&data)?;
    let public_inputs = circuit.prepare_public_inputs(&data)?;

    let circuit_settings = circuit.settings.clone();

    let params = load_params_cmd(srs_path, circuit_settings.run_args.logrows)?;

    let pk = load_pk::<KZGCommitmentScheme<Bn256>, Fr, GraphCircuit>(pk_path, circuit_settings)
        .map_err(Box::<dyn Error>::from)?;

    trace!("params computed");

    let now = Instant::now();

    // creates and verifies the proof
    let snark = match strategy {
        StrategyType::Single => {
            let strategy = KZGSingleStrategy::new(&params);
            create_proof_circuit_kzg(
                circuit,
                &params,
                public_inputs,
                &pk,
                transcript,
                strategy,
                check_mode,
            )?
        }
        StrategyType::Accum => {
            let strategy = AccumulatorStrategy::new(&params);
            create_proof_circuit_kzg(
                circuit,
                &params,
                public_inputs,
                &pk,
                transcript,
                strategy,
                check_mode,
            )?
        }
    };
    let elapsed = now.elapsed();
    info!(
        "proof took {}.{}",
        elapsed.as_secs(),
        elapsed.subsec_millis()
    );

    if let Some(proof_path) = proof_path {
        snark.save(&proof_path)?;
    }

    Ok(snark)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn fuzz(
    compiled_model_path: PathBuf,
    logrows: u32,
    data_path: PathBuf,
    transcript: TranscriptType,
    num_runs: usize,
    run_args: RunArgs,
    settings_path: Option<PathBuf>,
) -> Result<(), Box<dyn Error>> {
    check_solc_requirement();
    let passed = AtomicBool::new(true);

    info!("setting up tests");

    let _r = Gag::stdout().unwrap();
    let params = gen_srs::<KZGCommitmentScheme<Bn256>>(logrows);

    let data = GraphWitness::from_path(data_path)?;
    // these aren't real values so the sanity checks are mostly meaningless
    let mut circuit = match settings_path {
        Some(path) => {
            let circuit_settings = GraphSettings::load(&path)?;
            GraphCircuit::preprocessed_from_settings(
                &circuit_settings,
                &compiled_model_path,
                CheckMode::UNSAFE,
            )?
        }
        None => GraphCircuit::preprocessed_from_run_args(&run_args, &compiled_model_path)?,
    };

    let pk = create_keys::<KZGCommitmentScheme<Bn256>, Fr, GraphCircuit>(&circuit, &params)
        .map_err(Box::<dyn Error>::from)?;

    circuit.load_graph_witness(&data)?;
    let public_inputs = circuit.prepare_public_inputs(&data)?;

    let strategy = KZGSingleStrategy::new(&params);
    std::mem::drop(_r);

    info!("starting fuzzing");

    info!("fuzzing pk");

    let fuzz_pk = || {
        let new_params = gen_srs::<KZGCommitmentScheme<Bn256>>(logrows);

        let bad_pk =
            create_keys::<KZGCommitmentScheme<Bn256>, Fr, GraphCircuit>(&circuit, &new_params)
                .unwrap();

        let bad_proof = create_proof_circuit_kzg(
            circuit.clone(),
            &params,
            public_inputs.clone(),
            &bad_pk,
            transcript,
            strategy.clone(),
            CheckMode::UNSAFE,
        )
        .unwrap();

        verify_proof_circuit_kzg(
            params.verifier_params(),
            bad_proof,
            pk.get_vk(),
            strategy.clone(),
        )
        .map_err(|_| ())
    };

    run_fuzz_fn(num_runs, fuzz_pk, &passed);

    info!("fuzzing public inputs");

    let fuzz_public_inputs = || {
        let mut bad_inputs = vec![];
        for l in &public_inputs {
            bad_inputs.push(vec![Fr::random(rand::rngs::OsRng); l.len()]);
        }

        let bad_proof = create_proof_circuit_kzg(
            circuit.clone(),
            &params,
            bad_inputs.clone(),
            &pk,
            transcript,
            strategy.clone(),
            CheckMode::UNSAFE,
        )
        .unwrap();

        verify_proof_circuit_kzg(
            params.verifier_params(),
            bad_proof,
            pk.get_vk(),
            strategy.clone(),
        )
        .map_err(|_| ())
    };

    run_fuzz_fn(num_runs, fuzz_public_inputs, &passed);

    info!("fuzzing vk");

    let proof = create_proof_circuit_kzg(
        circuit.clone(),
        &params,
        public_inputs.clone(),
        &pk,
        transcript,
        strategy.clone(),
        CheckMode::SAFE,
    )?;

    let fuzz_vk = || {
        let new_params = gen_srs::<KZGCommitmentScheme<Bn256>>(logrows);

        let bad_pk =
            create_keys::<KZGCommitmentScheme<Bn256>, Fr, GraphCircuit>(&circuit, &new_params)
                .unwrap();

        let bad_vk = bad_pk.get_vk();

        verify_proof_circuit_kzg(
            params.verifier_params(),
            proof.clone(),
            bad_vk,
            strategy.clone(),
        )
        .map_err(|_| ())
    };

    run_fuzz_fn(num_runs, fuzz_vk, &passed);

    info!("fuzzing proof bytes");

    let fuzz_proof_bytes = || {
        let mut rng = rand::thread_rng();

        let bad_proof_bytes: Vec<u8> = (0..proof.proof.len())
            .map(|_| rng.gen_range(0..20))
            .collect();

        let bad_proof = Snark::<_, _> {
            instances: proof.instances.clone(),
            proof: bad_proof_bytes,
            protocol: proof.protocol.clone(),
            transcript_type: transcript,
        };

        verify_proof_circuit_kzg(
            params.verifier_params(),
            bad_proof,
            pk.get_vk(),
            strategy.clone(),
        )
        .map_err(|_| ())
    };

    run_fuzz_fn(num_runs, fuzz_proof_bytes, &passed);

    info!("fuzzing proof instances");

    let fuzz_proof_instances = || {
        let mut bad_inputs = vec![];
        for l in &proof.instances {
            bad_inputs.push(vec![Fr::random(rand::rngs::OsRng); l.len()]);
        }

        let bad_proof = Snark::<_, _> {
            instances: bad_inputs.clone(),
            proof: proof.proof.clone(),
            protocol: proof.protocol.clone(),
            transcript_type: transcript,
        };

        verify_proof_circuit_kzg(
            params.verifier_params(),
            bad_proof,
            pk.get_vk(),
            strategy.clone(),
        )
        .map_err(|_| ())
    };

    run_fuzz_fn(num_runs, fuzz_proof_instances, &passed);

    if matches!(transcript, TranscriptType::EVM) {
        let num_instance = circuit.settings.total_instances();

        let yul_code = gen_evm_verifier(&params, pk.get_vk(), num_instance)?;
        let deployment_code = gen_deployment_code(yul_code).unwrap();

        info!("fuzzing proof bytes for evm verifier");

        let fuzz_evm_proof_bytes = || {
            let mut rng = rand::thread_rng();

            let bad_proof_bytes: Vec<u8> = (0..proof.proof.len())
                .map(|_| rng.gen_range(0..20))
                .collect();

            let bad_proof = Snark::<_, _> {
                instances: proof.instances.clone(),
                proof: bad_proof_bytes,
                protocol: proof.protocol.clone(),
                transcript_type: transcript,
            };

            let res = evm_verify(deployment_code.clone(), bad_proof);

            match res {
                Ok(_) => Ok(()),
                Err(_) => Err(()),
            }
        };

        run_fuzz_fn(num_runs, fuzz_evm_proof_bytes, &passed);

        info!("fuzzing proof instances for evm verifier");

        let fuzz_evm_instances = || {
            let mut bad_inputs = vec![];
            for l in &proof.instances {
                bad_inputs.push(vec![Fr::random(rand::rngs::OsRng); l.len()]);
            }

            let bad_proof = Snark::<_, _> {
                instances: bad_inputs.clone(),
                proof: proof.proof.clone(),
                protocol: proof.protocol.clone(),
                transcript_type: transcript,
            };

            let res = evm_verify(deployment_code.clone(), bad_proof);

            match res {
                Ok(_) => Ok(()),
                Err(_) => Err(()),
            }
        };

        run_fuzz_fn(num_runs, fuzz_evm_instances, &passed);
    }

    if !passed.into_inner() {
        Err("fuzzing failed".into())
    } else {
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn run_fuzz_fn(
    num_runs: usize,
    f: impl Fn() -> Result<(), ()> + std::marker::Sync + std::marker::Send,
    passed: &AtomicBool,
) {
    let num_failures = AtomicI64::new(0);
    let _r = Gag::stdout().unwrap();

    let pb = init_bar(num_runs as u64);
    pb.set_message("fuzzing...");
    (0..num_runs).into_par_iter().for_each(|_| {
        let result = f();
        if result.is_ok() {
            passed.swap(false, Ordering::Relaxed);
            num_failures.fetch_add(1, Ordering::Relaxed);
        }
        pb.inc(1);
    });
    pb.finish_with_message("Done.");
    std::mem::drop(_r);
    info!(
        "num failures: {} out of {}",
        num_failures.load(Ordering::Relaxed),
        num_runs
    );
}

pub(crate) fn mock_aggregate(
    aggregation_snarks: Vec<PathBuf>,
    logrows: u32,
) -> Result<(), Box<dyn Error>> {
    let mut snarks = vec![];
    for proof_path in aggregation_snarks.iter() {
        snarks.push(Snark::load::<KZGCommitmentScheme<Bn256>>(proof_path)?);
    }
    // proof aggregation
    #[cfg(not(target_arch = "wasm32"))]
    let pb = {
        let pb = init_spinner();
        pb.set_message("Aggregating (may take a while)...");
        pb
    };

    let circuit = AggregationCircuit::new(&G1Affine::generator().into(), snarks)?;

    let prover = halo2_proofs::dev::MockProver::run(logrows, &circuit, circuit.instances())
        .map_err(Box::<dyn Error>::from)?;
    prover
        .verify_par()
        .map_err(|e| Box::<dyn Error>::from(ExecutionError::VerifyError(e)))?;
    #[cfg(not(target_arch = "wasm32"))]
    pb.finish_with_message("Done.");
    Ok(())
}

pub(crate) fn setup_aggregate(
    sample_snarks: Vec<PathBuf>,
    vk_path: PathBuf,
    pk_path: PathBuf,
    srs_path: PathBuf,
    logrows: u32,
) -> Result<(), Box<dyn Error>> {
    // the K used for the aggregation circuit
    let params = load_params_cmd(srs_path, logrows)?;

    let mut snarks = vec![];
    for proof_path in sample_snarks.iter() {
        snarks.push(Snark::load::<KZGCommitmentScheme<Bn256>>(proof_path)?);
    }

    let agg_circuit = AggregationCircuit::new(&params.get_g()[0].into(), snarks)?;
    let agg_pk =
        create_keys::<KZGCommitmentScheme<Bn256>, Fr, AggregationCircuit>(&agg_circuit, &params)?;

    let agg_vk = agg_pk.get_vk();

    // now save
    save_vk::<KZGCommitmentScheme<Bn256>>(&vk_path, agg_vk)?;
    save_pk::<KZGCommitmentScheme<Bn256>>(&pk_path, &agg_pk)?;
    Ok(())
}

pub(crate) fn aggregate(
    proof_path: PathBuf,
    aggregation_snarks: Vec<PathBuf>,
    pk_path: PathBuf,
    srs_path: PathBuf,
    transcript: TranscriptType,
    logrows: u32,
    check_mode: CheckMode,
) -> Result<(), Box<dyn Error>> {
    // the K used for the aggregation circuit
    let params = load_params_cmd(srs_path, logrows)?;

    let mut snarks = vec![];
    for proof_path in aggregation_snarks.iter() {
        snarks.push(Snark::load::<KZGCommitmentScheme<Bn256>>(proof_path)?);
    }

    let agg_pk = load_pk::<KZGCommitmentScheme<Bn256>, Fr, AggregationCircuit>(pk_path, ())?;
    // proof aggregation
    #[cfg(not(target_arch = "wasm32"))]
    let pb = {
        let pb = init_spinner();
        pb.set_message("Aggregating (may take a while)...");
        pb
    };

    {
        let agg_circuit = AggregationCircuit::new(&params.get_g()[0].into(), snarks)?;

        let now = Instant::now();
        let snark = create_proof_circuit_kzg(
            agg_circuit.clone(),
            &params,
            agg_circuit.instances(),
            &agg_pk,
            transcript,
            AccumulatorStrategy::new(&params),
            check_mode,
        )?;

        let elapsed = now.elapsed();
        info!(
            "Aggregation proof took {}.{}",
            elapsed.as_secs(),
            elapsed.subsec_millis()
        );
        snark.save(&proof_path)?;
    }
    #[cfg(not(target_arch = "wasm32"))]
    pb.finish_with_message("Done.");

    Ok(())
}

pub(crate) fn verify(
    proof_path: PathBuf,
    settings_path: PathBuf,
    vk_path: PathBuf,
    srs_path: PathBuf,
) -> Result<(), Box<dyn Error>> {
    let circuit_settings = GraphSettings::load(&settings_path)?;
    let params = load_params_cmd(srs_path, circuit_settings.run_args.logrows)?;
    let proof = Snark::load::<KZGCommitmentScheme<Bn256>>(&proof_path)?;

    let strategy = KZGSingleStrategy::new(params.verifier_params());
    let vk = load_vk::<KZGCommitmentScheme<Bn256>, Fr, GraphCircuit>(vk_path, circuit_settings)?;
    let now = Instant::now();
    let result = verify_proof_circuit_kzg(params.verifier_params(), proof, &vk, strategy);
    let elapsed = now.elapsed();
    info!(
        "verify took {}.{}",
        elapsed.as_secs(),
        elapsed.subsec_millis()
    );
    info!("verified: {}", result.is_ok());
    result.map_err(|e| e.into())
}

pub(crate) fn verify_aggr(
    proof_path: PathBuf,
    vk_path: PathBuf,
    srs_path: PathBuf,
    logrows: u32,
) -> Result<(), Box<dyn Error>> {
    let params = load_params_cmd(srs_path, logrows)?;

    let proof = Snark::load::<KZGCommitmentScheme<Bn256>>(&proof_path)?;

    let strategy = AccumulatorStrategy::new(params.verifier_params());
    let vk = load_vk::<KZGCommitmentScheme<Bn256>, Fr, AggregationCircuit>(vk_path, ())?;
    let now = Instant::now();
    let result = verify_proof_circuit_kzg(&params, proof, &vk, strategy);

    let elapsed = now.elapsed();
    info!(
        "verify took {}.{}",
        elapsed.as_secs(),
        elapsed.subsec_millis()
    );
    info!("verified: {}", result.is_ok());
    result?;
    Ok(())
}

/// helper function for load_params
pub(crate) fn load_params_cmd(
    srs_path: PathBuf,
    logrows: u32,
) -> Result<ParamsKZG<Bn256>, Box<dyn Error>> {
    let mut params: ParamsKZG<Bn256> = load_srs::<KZGCommitmentScheme<Bn256>>(srs_path)?;
    info!("downsizing params to {} logrows", logrows);
    if logrows < params.k() {
        params.downsize(logrows);
    }
    Ok(params)
}
