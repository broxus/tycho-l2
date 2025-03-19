use std::future;

use anyhow::{Context, Result};
use clap::Parser;
use tycho_core::block_strider::{
    ArchiveBlockProvider, BlockProviderExt, BlockSubscriber, BlockSubscriberContext,
    BlockchainBlockProvider, ColdBootType, StorageBlockProvider,
};
use tycho_util::cli::signal;

#[derive(Parser)]
pub struct Cmd {
    #[clap(flatten)]
    pub base: tycho_light_node::CmdRun,
}

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

        let node_config = NodeConfig::from_file(self.base.config.as_ref().context("no config")?)?;
        // TODO: Impl `init_config`.

        if let Some(metrics) = &node_config.metrics {
            tycho_util::cli::metrics::init_metrics(&metrics)?;
        }

        rayon::ThreadPoolBuilder::new()
            .stack_size(8 * 1024 * 1024)
            .thread_name(|_| "rayon_worker".to_string())
            .num_threads(node_config.threads.rayon_threads)
            .build_global()
            .unwrap();

        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(node_config.threads.tokio_workers)
            .build()?
            .block_on(async move {
                let run_fut = tokio::spawn(self.run_impl(node_config));
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

    async fn run_impl(self, node_config: NodeConfig) -> Result<()> {
        let import_zerostate = self.base.import_zerostate.clone();

        let mut node = self.base.create(node_config.clone()).await?;

        let archive_block_provider = ArchiveBlockProvider::new(
            node.blockchain_rpc_client().clone(),
            node.storage().clone(),
            node_config.archive_block_provider.clone(),
        );

        let storage_block_provider = StorageBlockProvider::new(node.storage().clone());

        let blockchain_block_provider = BlockchainBlockProvider::new(
            node.blockchain_rpc_client().clone(),
            node.storage().clone(),
            node_config.blockchain_block_provider.clone(),
        )
        .with_fallback(archive_block_provider.clone());

        let init_block_id = node
            .init(ColdBootType::LatestPersistent, import_zerostate)
            .await?;
        node.update_validator_set(&init_block_id).await?;
        node.run(
            archive_block_provider.chain((blockchain_block_provider, storage_block_provider)),
            PrintSubscriber,
        )
        .await?;

        futures_util::future::pending().await
    }
}

#[derive(Default, Debug, Clone, Copy)]
pub struct PrintSubscriber;

impl BlockSubscriber for PrintSubscriber {
    type Prepared = ();

    type PrepareBlockFut<'a> = future::Ready<Result<()>>;
    type HandleBlockFut<'a> = future::Ready<Result<()>>;

    fn prepare_block<'a>(&'a self, cx: &'a BlockSubscriberContext) -> Self::PrepareBlockFut<'a> {
        tracing::info!(
            block_id = %cx.block.id(),
            mc_block_id = %cx.mc_block_id,
            "preparing block"
        );
        future::ready(Ok(()))
    }

    fn handle_block(
        &self,
        cx: &BlockSubscriberContext,
        _: Self::Prepared,
    ) -> Self::HandleBlockFut<'_> {
        tracing::info!(
            block_id = %cx.block.id(),
            mc_block_id = %cx.mc_block_id,
            "handling block"
        );
        future::ready(Ok(()))
    }
}

type NodeConfig = tycho_light_node::NodeConfig<()>;
