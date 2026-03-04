/// HuntLoan execution engine — EIP-1559 tx construction, signing, nonce
/// management, retry with fee-bumping, LIVE vs DRY_RUN modes.
///
/// Pipeline position: scanner → simulator → [executor] → contract
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
    signers::local::PrivateKeySigner,
    sol,
    sol_types::SolCall,
};

/// Concrete type returned by `ProviderBuilder::new().wallet(w).connect_http(url)`.
type DefaultFillers = JoinFill<
    Identity,
    JoinFill<GasFiller, JoinFill<BlobGasFiller, JoinFill<NonceFiller, ChainIdFiller>>>,
>;
type WalletHttpProvider = FillProvider<
    JoinFill<DefaultFillers, WalletFiller<EthereumWallet>>,
    RootProvider,
>;
use eyre::{bail, Result, WrapErr};
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::{config::Config, gas, scanner::Opportunity, simulator::SimOutput};

// Solidity interface — matches HuntLoanFlashReceiver.sol
sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    interface IHuntLoanReceiver {
        function requestFlashLiquidation(
            address debtAsset,
            uint256 debtAmount,
            address collateralAsset,
            address borrower
        ) external;
    }
}

/// Outcome of a successful execution.
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub tx_hash:         TxHash,
    pub block_number:    u64,
    pub gas_used:        u64,
    /// Wall-clock ms from execute() call to confirmed receipt.
    #[allow(dead_code)]
    pub send_latency_ms: u64,
}

/// Pre-calculated EIP-1559 fees for one tx attempt.
#[derive(Debug, Clone)]
struct TxFees {
    max_fee_per_gas:  u128,
    max_priority_fee: u128,
    gas_limit:        u64,
}

/// HuntLoan transaction executor.
///
/// Create once at startup and reuse across all blocks.
pub struct HuntLoanExecutor {
    config:          Arc<Config>,
    wallet:          EthereumWallet,
    signer_address:  Address,
    /// Cached nonce — bumped optimistically; reset from chain on error.
    nonce:           Mutex<Option<u64>>,
    /// Pre-built HTTP provider for tx submission — avoids rebuilding per-execution.
    submit_provider: Arc<WalletHttpProvider>,
}

impl HuntLoanExecutor {
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

    /// Execute a liquidation opportunity.
    ///
    /// DRY_RUN=true  → logs intent, no tx sent.
    /// DRY_RUN=false → broadcasts with up to 3 retry attempts, bumping fees +15%
    ///                 per retry using the same nonce.
    pub async fn execute(
        &self,
        opp: &Opportunity,
        sim: &SimOutput,
        base_fee_wei: u128,
        regime: gas::Regime,
        gross_profit_wei: u128,
    ) -> Result<ExecutionResult> {
        let t = Instant::now();

        if self.config.dry_run {
            let fees = self.compute_fees(
                opp.health_factor, base_fee_wei, sim.estimated_gas, regime, gross_profit_wei,
            );
            let tip_total_eth = fees.max_priority_fee as f64 * fees.gas_limit as f64 / 1e18;
            info!(
                mode              = "DRY_RUN",
                borrower          = %opp.borrower,
                debt_usd          = opp.debt_usd,
                estimated_profit  = sim.net_profit_usd,
                gross_profit_wei  = gross_profit_wei,
                max_fee_gwei      = fees.max_fee_per_gas / 1_000_000_000,
                priority_fee_gwei = fees.max_priority_fee / 1_000_000_000,
                tip_total_eth     = %format!("{:.6}", tip_total_eth),
                gas_limit         = fees.gas_limit,
                "DRY_RUN — tx NOT sent"
            );
            return Ok(ExecutionResult {
                tx_hash:         TxHash::ZERO,
                block_number:    0,
                gas_used:        0,
                send_latency_ms: t.elapsed().as_millis() as u64,
            });
        }

        // ── SOFT_LIVE: resolve nonce, encode calldata, print full tx preview ──
        if self.config.soft_live {
            let fees = self.compute_fees(
                opp.health_factor, base_fee_wei, sim.estimated_gas, regime, gross_profit_wei,
            );
            let provider = ProviderBuilder::new()
                .wallet(self.wallet.clone())
                .connect_http(self.config.rpc_http.parse().wrap_err("RPC_URL invalid")?);
            let nonce = self.acquire_nonce(&provider).await.unwrap_or(u64::MAX);

            let calldata = Bytes::from(
                IHuntLoanReceiver::requestFlashLiquidationCall {
                    debtAsset:        opp.debt_asset,
                    debtAmount:       U256::from(opp.debt_to_repay_raw),
                    collateralAsset:  opp.collateral_asset,
                    borrower:         opp.borrower,
                }
                .abi_encode(),
            );

            let tip_total_eth = fees.max_priority_fee as f64 * fees.gas_limit as f64 / 1e18;
            info!(
                mode                 = "SOFT_LIVE",
                to                   = %self.config.huntloan_addr,
                chain_id             = self.config.chain_id,
                nonce                = nonce,
                max_fee_gwei         = fees.max_fee_per_gas / 1_000_000_000,
                priority_fee_gwei    = fees.max_priority_fee / 1_000_000_000,
                tip_total_eth        = %format!("{:.6}", tip_total_eth),
                gross_profit_wei     = gross_profit_wei,
                gas_limit            = fees.gas_limit,
                calldata_bytes       = calldata.len(),
                calldata             = %calldata,
                borrower             = %opp.borrower,
                debt_to_repay_usd    = opp.debt_to_repay,
                debt_to_repay_raw    = opp.debt_to_repay_raw,
                collateral           = %opp.collateral_asset,
                debt_asset           = %opp.debt_asset,
                estimated_profit_usd = sim.net_profit_usd,
                "SOFT_LIVE — full tx preview (NOT broadcast)"
            );
            return Ok(ExecutionResult {
                tx_hash:         TxHash::ZERO,
                block_number:    0,
                gas_used:        0,
                send_latency_ms: t.elapsed().as_millis() as u64,
            });
        }

        // ── LIVE: RBF escalation loop ────────────────────────────────────────
        self.broadcast_with_retry(
            opp, base_fee_wei, regime, gross_profit_wei,
            opp.health_factor, sim.estimated_gas, t,
        ).await
    }

    // ── Broadcast + RBF escalation ──────────────────────────────────────────

    async fn broadcast_with_retry(
        &self,
        opp: &Opportunity,
        base_fee_wei: u128,
        regime: gas::Regime,
        gross_profit_wei: u128,
        health_factor: f64,
        est_gas: u64,
        t: Instant,
    ) -> Result<ExecutionResult> {
        use crate::constants::{RBF_BRIBE_STEPS, RBF_WAIT_MS};

        // Use cached provider (private RPC for MEV protection)
        let provider = self.submit_provider.clone();

        // Micro-bankroll gate: refuse broadcast if wallet is below safety floor.
        let balance_wei = provider
            .get_balance(self.signer_address)
            .await
            .map(|b| b.to::<u128>())
            .unwrap_or(u128::MAX);
        if balance_wei < self.config.min_wallet_eth_wei {
            bail!(
                "Wallet {:.6} ETH below safety floor {:.6} ETH — refusing broadcast to preserve capital",
                balance_wei as f64 / 1e18,
                self.config.min_wallet_eth_wei as f64 / 1e18,
            );
        }

        // Build escalation steps: RBF_BRIBE_STEPS + config.max_bribe_fraction as final attempt
        let mut steps: Vec<f64> = RBF_BRIBE_STEPS.to_vec();
        steps.push(self.config.max_bribe_fraction);
        let num_steps = steps.len();

        let nonce = self.acquire_nonce(&provider).await?;

        let gas_limit = if est_gas > 0 {
            ((est_gas as u128 * 120 / 100).max(800_000)) as u64
        } else {
            800_000_u64
        };

        for (attempt, &bribe_frac) in steps.iter().enumerate() {
            let gas_tier = gas::compute_profit_aware_fees_with_bribe(
                base_fee_wei, gross_profit_wei, health_factor,
                regime, est_gas, self.config.max_bribe_wei, bribe_frac,
            );

            let fees = TxFees {
                max_fee_per_gas:  gas_tier.max_fee_per_gas,
                max_priority_fee: gas_tier.max_priority_fee,
                gas_limit,
            };

            let tip_total_eth = fees.max_priority_fee as f64 * fees.gas_limit as f64 / 1e18;
            info!(
                attempt           = attempt + 1,
                bribe_fraction    = %format!("{:.2}", bribe_frac),
                borrower          = %opp.borrower,
                max_fee_gwei      = fees.max_fee_per_gas / 1_000_000_000,
                priority_fee_gwei = fees.max_priority_fee / 1_000_000_000,
                tip_total_eth     = %format!("{:.6}", tip_total_eth),
                nonce             = nonce,
                "Broadcasting liquidation tx (RBF)"
            );

            let contract = IHuntLoanReceiver::new(self.config.huntloan_addr, &provider);

            let call = contract
                .requestFlashLiquidation(
                    opp.debt_asset,
                    U256::from(opp.debt_to_repay_raw),
                    opp.collateral_asset,
                    opp.borrower,
                )
                .max_fee_per_gas(fees.max_fee_per_gas)
                .max_priority_fee_per_gas(fees.max_priority_fee)
                .gas(fees.gas_limit)
                .nonce(nonce);

            let is_last = attempt + 1 == num_steps;

            match call.send().await {
                Ok(pending) => {
                    let tx_hash = *pending.tx_hash();
                    info!(tx_hash = %tx_hash, attempt = attempt + 1, "Tx submitted");

                    if is_last {
                        // Last attempt — wait for receipt directly
                        match pending.get_receipt().await {
                            Ok(receipt) => return self.handle_receipt(receipt, tx_hash, nonce, t).await,
                            Err(e) => {
                                warn!(attempt = attempt + 1, error = %e, "Receipt wait failed on final attempt");
                            }
                        }
                    } else {
                        // Non-last: race receipt vs escalation timer
                        tokio::select! {
                            receipt_result = pending.get_receipt() => {
                                match receipt_result {
                                    Ok(receipt) => return self.handle_receipt(receipt, tx_hash, nonce, t).await,
                                    Err(e) => {
                                        warn!(attempt = attempt + 1, error = %e, "Receipt wait failed — escalating");
                                    }
                                }
                            }
                            _ = tokio::time::sleep(Duration::from_millis(RBF_WAIT_MS)) => {
                                info!(attempt = attempt + 1, "RBF timer fired — escalating bribe");
                                // continue to next step
                            }
                        }
                    }
                }
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("revert") || msg.contains("execution reverted") {
                        self.invalidate_nonce().await;
                        bail!("Tx reverted: {}", msg);
                    }
                    warn!(attempt = attempt + 1, error = %msg, "Send error — escalating immediately");
                    // Skip wait, escalate immediately
                }
            }
        }

        self.invalidate_nonce().await;
        bail!("Execution failed after {} RBF attempts", num_steps)
    }

    /// Extract receipt data, confirm nonce, log, and return ExecutionResult.
    async fn handle_receipt(
        &self,
        receipt: alloy::rpc::types::TransactionReceipt,
        tx_hash: TxHash,
        nonce: u64,
        t: Instant,
    ) -> Result<ExecutionResult> {
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

    // ── EIP-1559 fee computation ──────────────────────────────────────────────

    fn compute_fees(
        &self,
        health_factor: f64,
        base_fee_wei: u128,
        est_gas: u64,
        regime: gas::Regime,
        gross_profit_wei: u128,
    ) -> TxFees {
        let gas_tier = gas::compute_profit_aware_fees(
            base_fee_wei,
            gross_profit_wei,
            health_factor,
            regime,
            est_gas,
            self.config.max_bribe_wei,
            self.config.max_bribe_fraction,
        );

        let gas_limit = if est_gas > 0 {
            ((est_gas as u128 * 120 / 100).max(800_000)) as u64
        } else {
            800_000_u64
        };

        TxFees {
            max_fee_per_gas:  gas_tier.max_fee_per_gas,
            max_priority_fee: gas_tier.max_priority_fee,
            gas_limit,
        }
    }

    /// Pre-fetch nonce from chain into cache. Call while sims are running
    /// to save ~20-30ms from the execution critical path.
    pub async fn prefetch_nonce(&self) {
        let provider = self.submit_provider.clone();
        let _ = self.acquire_nonce(provider.as_ref()).await;
    }

    // ── Nonce management ─────────────────────────────────────────────────────

    async fn acquire_nonce<P: Provider>(&self, provider: &P) -> Result<u64> {
        // Fast path: cached nonce available, short lock
        {
            let mut guard = self.nonce.lock().await;
            if let Some(n) = *guard {
                *guard = Some(n + 1);
                return Ok(n);
            }
        }
        // Cold path: RPC call outside lock to avoid blocking other tasks
        let on_chain = provider
            .get_transaction_count(self.signer_address)
            .await
            .wrap_err("get_transaction_count failed")?;
        // Re-check under lock (another task may have populated it)
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

    async fn invalidate_nonce(&self) {
        *self.nonce.lock().await = None;
    }
}
