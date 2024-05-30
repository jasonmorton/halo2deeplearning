// ignore file if compiling for wasm

#[cfg(not(target_arch = "wasm32"))]
use clap::{CommandFactory, Parser};
#[cfg(not(target_arch = "wasm32"))]
use colored_json::ToColoredJson;
#[cfg(not(target_arch = "wasm32"))]
use ezkl::commands::Cli;
#[cfg(not(target_arch = "wasm32"))]
use ezkl::execute::run;
#[cfg(not(target_arch = "wasm32"))]
use ezkl::logger::init_logger;
#[cfg(not(target_arch = "wasm32"))]
use log::{error, info};
#[cfg(not(any(target_arch = "wasm32", feature = "no-banner")))]
use rand::prelude::SliceRandom;
#[cfg(not(target_arch = "wasm32"))]
#[cfg(feature = "icicle")]
use std::env;
#[cfg(not(target_arch = "wasm32"))]
use std::error::Error;

#[tokio::main(flavor = "current_thread")]
#[cfg(not(target_arch = "wasm32"))]
pub async fn main() -> Result<(), Box<dyn Error>> {
    let args = Cli::parse();
    init_logger();
    #[cfg(not(any(target_arch = "wasm32", feature = "no-banner")))]
    banner();

    if let Some(generator) = args.generator {
        ezkl::commands::print_completions(generator, &mut Cli::command());
        Ok(())
    } else if let Some(command) = args.command {
        #[cfg(feature = "icicle")]
        if env::var("ENABLE_ICICLE_GPU").is_ok() {
            info!("Running with ICICLE GPU");
        } else {
            info!("Running with CPU");
        }
        info!("command: \n {}", &command.as_json().to_colored_json_auto()?);
        let res = run(command).await;
        match &res {
            Ok(_) => info!("succeeded"),
            Err(e) => error!("failed: {}", e),
        };
        res.map(|_| ())
    } else {
        Err("No command provided".into())
    }
}

#[cfg(target_arch = "wasm32")]
pub fn main() {}

#[cfg(not(any(target_arch = "wasm32", feature = "no-banner")))]
fn banner() {
    let ell: Vec<&str> = vec![
        "for Neural Networks",
        "Linear Algebra",
        "for Layers",
        "for the Laconic",
        "Learning",
        "for Liberty",
        "for the Lyrical",
    ];
    info!(
        "{}",
        format!(
            "

        ███████╗███████╗██╗  ██╗██╗
        ██╔════╝╚══███╔╝██║ ██╔╝██║
        █████╗    ███╔╝ █████╔╝ ██║
        ██╔══╝   ███╔╝  ██╔═██╗ ██║
        ███████╗███████╗██║  ██╗███████╗
        ╚══════╝╚══════╝╚═╝  ╚═╝╚══════╝

        -----------------------------------------------------------
        Easy Zero Knowledge {}.
        -----------------------------------------------------------

        ",
            ell.choose(&mut rand::thread_rng()).unwrap()
        )
    );
}
