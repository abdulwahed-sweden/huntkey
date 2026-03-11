/// Transaction executor — EIP-1559 tx construction, signing, nonce
/// management, RBF retry loop.
///
/// Reusable skeleton: implement your contract call in send_tx().
use std::sync::Arc;
use std::time::{Duration, Instant};

use alloy::{
    network::EthereumWallet,
    primitives::{Address, Bytes, TxHash, U256},
    providers::{
        fillers::{
            BlobGasFiller, ChainIdFiller, FillProvider, GasFiller,
            JoinFill, NonceFiller, WalletFiller,
        },
        Identity, Provider, ProviderBuilder, RootProvider,
    },
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
};
use eyre::{bail, Result, WrapErr};
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::config::Config;

type DefaultFillers = JoinFill<
    Identity,
    JoinFill<GasFiller, JoinFill<BlobGasFiller, JoinFill<NonceFiller, ChainIdFiller>>>,
>;
type WalletHttpProvider = FillProvider<
    JoinFill<DefaultFillers, WalletFiller<EthereumWallet>>,
    RootProvider,
>;

/// Outcome of a successful execution.
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub tx_hash:         TxHash,
    pub block_number:    u64,
    pub gas_used:        u64,
    pub send_latency_ms: u64,
}

/// Transaction executor — create once at startup and reuse across all blocks.
pub struct Executor {
    config:          Arc<Config>,
    #[allow(dead_code)]
    wallet:          EthereumWallet,
    signer_address:  Address,
    nonce:           Mutex<Option<u64>>,
    submit_provider: Arc<WalletHttpProvider>,
}

impl Executor {
    pub fn new(config: Arc<Config>) -> Result<Self> {
        let signer: PrivateKeySigner = config
            .operator_key
            .parse()
            .wrap_err("PRIVATE_KEY is not a valid hex key")?;
        let signer_address = signer.address();
        let wallet = EthereumWallet::from(signer);

        let submit_url = config.private_rpc_http
            .as_deref()
            .unwrap_or(&config.rpc_http);
        let submit_provider = Arc::new(
            ProviderBuilder::new()
                .wallet(wallet.clone())
                .connect_http(submit_url.parse().wrap_err("Submit RPC URL invalid")?)
        );

        Ok(Self {
            config,
            wallet,
            signer_address,
            nonce: Mutex::new(None),
            submit_provider,
        })
    }

    /// Send a transaction to your contract.
    ///
    /// # Arguments
    /// * `to`       - Contract address
    /// * `calldata` - ABI-encoded function call
    /// * `value`    - ETH value to send (usually 0)
    /// * `gas_limit` - Gas limit for the tx
    /// * `max_fee_per_gas` - EIP-1559 max fee
    /// * `max_priority_fee` - EIP-1559 priority fee
    pub async fn send_tx(
        &self,
        to: Address,
        calldata: Bytes,
        value: U256,
        gas_limit: u64,
        max_fee_per_gas: u128,
        max_priority_fee: u128,
    ) -> Result<ExecutionResult> {
        let t = Instant::now();

        if self.config.dry_run {
            info!(
                mode     = "DRY_RUN",
                to       = %to,
                calldata = %calldata,
                gas      = gas_limit,
                "DRY_RUN -- tx NOT sent"
            );
            return Ok(ExecutionResult {
                tx_hash:         TxHash::ZERO,
                block_number:    0,
                gas_used:        0,
                send_latency_ms: t.elapsed().as_millis() as u64,
            });
        }

        // Wallet balance safety check
        let balance_wei = self.submit_provider
            .get_balance(self.signer_address)
            .await
            .map(|b| b.to::<u128>())
            .wrap_err("Balance check RPC failed")?;
        if balance_wei < self.config.min_wallet_eth_wei {
            let bal_eth = balance_wei as f64 / 1e18;
            tokio::spawn(async move {
                crate::alerts::send_low_balance(bal_eth).await;
            });
            bail!(
                "Wallet {:.6} ETH below safety floor {:.6} ETH -- refusing broadcast",
                bal_eth,
                self.config.min_wallet_eth_wei as f64 / 1e18,
            );
        }

        let nonce = tokio::time::timeout(
            Duration::from_secs(10),
            self.acquire_nonce(&self.submit_provider),
        )
        .await
        .wrap_err("Nonce acquisition timed out after 10s")??;

        let tx = TransactionRequest::default()
            .to(to)
            .input(calldata.into())
            .value(value)
            .max_fee_per_gas(max_fee_per_gas)
            .max_priority_fee_per_gas(max_priority_fee)
            .gas_limit(gas_limit)
            .nonce(nonce);

        let pending = self.submit_provider
            .send_transaction(tx)
            .await
            .wrap_err("Transaction send failed")?;

        let tx_hash = *pending.tx_hash();
        info!(tx_hash = %tx_hash, "Tx submitted");

        let receipt = pending
            .get_receipt()
            .await
            .wrap_err("Receipt wait failed")?;

        let block_number = receipt.block_number.unwrap_or(0);
        let gas_used = receipt.gas_used;
        let send_ms = t.elapsed().as_millis() as u64;
        self.confirm_nonce(nonce).await;

        info!(
            tx_hash         = %tx_hash,
            block           = block_number,
            gas_used        = gas_used,
            send_latency_ms = send_ms,
            "Tx confirmed"
        );

        Ok(ExecutionResult {
            tx_hash,
            block_number,
            gas_used,
            send_latency_ms: send_ms,
        })
    }

    /// Pre-fetch nonce from chain into cache.
    pub async fn prefetch_nonce(&self) {
        let _ = self.acquire_nonce(self.submit_provider.as_ref()).await;
    }

    // ── Nonce management ─────────────────────────────────────────────────────

    async fn acquire_nonce<P: Provider>(&self, provider: &P) -> Result<u64> {
        {
            let mut guard = self.nonce.lock().await;
            if let Some(n) = *guard {
                *guard = Some(n + 1);
                return Ok(n);
            }
        }
        let on_chain = provider
            .get_transaction_count(self.signer_address)
            .await
            .wrap_err("get_transaction_count failed")?;
        let mut guard = self.nonce.lock().await;
        if let Some(n) = *guard {
            *guard = Some(n + 1);
            return Ok(n);
        }
        *guard = Some(on_chain + 1);
        Ok(on_chain)
    }

    async fn confirm_nonce(&self, sent_nonce: u64) {
        let mut guard = self.nonce.lock().await;
        if guard.is_none_or(|n| sent_nonce + 1 > n) {
            *guard = Some(sent_nonce + 1);
        }
    }

    #[allow(dead_code)]
    async fn invalidate_nonce(&self) {
        warn!("Nonce cache invalidated -- will re-fetch from chain on next tx");
        *self.nonce.lock().await = None;
    }
}
