use anyhow::Result;
use clap::Parser;
use tycho_util::cli::signal;

#[derive(Parser)]
pub struct Cmd {}

impl Cmd {
    pub fn run(self) -> Result<()> {
        std::panic::set_hook(Box::new(|info| {
            use std::io::Write;
            let backtrace = std::backtrace::Backtrace::capture();

            tracing::error!("{info}\n{backtrace}");
            std::io::stderr().flush().ok();
            std::io::stdout().flush().ok();
            std::process::exit(1);
        }));

        rayon::ThreadPoolBuilder::new()
            .stack_size(8 * 1024 * 1024)
            .thread_name(|_| "rayon_worker".to_string())
            .num_threads(2) // TODO: move to config
            .build_global()
            .unwrap();

        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2) // TODO: move to config
            .build()?
            .block_on(async move {
                let run_fut = tokio::spawn(self.run_impl());
                let stop_fut = signal::any_signal(signal::TERMINATION_SIGNALS);
                tokio::select! {
                    res = run_fut => res.unwrap(),
                    signal = stop_fut => match signal {
                        Ok(signal) => {
                            tracing::info!(?signal, "received termination signal");
                            Ok(())
                        }
                        Err(e) => Err(e.into()),
                    }
                }
            })
    }

    async fn run_impl(self) -> Result<()> {
        // TODO:

        Ok(())
    }
}
