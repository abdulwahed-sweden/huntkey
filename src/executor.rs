/// HuntLoan execution engine — EIP-1559 tx construction, signing, nonce
/// management, retry with fee-bumping, LIVE vs DRY_RUN modes.
///
/// Pipeline position: scanner → simulator → [executor] → contract
use std::sync::Arc;
use std::time::Instant;

use alloy::{
    network::EthereumWallet,
    primitives::{Address, Bytes, TxHash, U256},
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
    sol,
    sol_types::SolCall,
};
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
    config:         Arc<Config>,
    wallet:         EthereumWallet,
    signer_address: Address,
    /// Cached nonce — bumped optimistically; reset from chain on error.
    nonce:          Mutex<Option<u64>>,
}

impl HuntLoanExecutor {
    pub fn new(config: Arc<Config>) -> Result<Self> {
        let signer: PrivateKeySigner = config
            .operator_key
            .parse()
            .wrap_err("PRIVATE_KEY is not a valid hex key")?;
        let signer_address = signer.address();
        let wallet = EthereumWallet::from(signer);
        Ok(Self {
            config,
            wallet,
            signer_address,
            nonce: Mutex::new(None),
        })
    }

    /// Fire two transactions simultaneously for high-conviction opportunities.
    ///
    /// Shot 1 (nonce N):   STRIKE tier fees — competitive baseline.
    /// Shot 2 (nonce N+1): KILL tier fees   — aggressive escalation.
    ///
    /// Both are sent before either is awaited, maximising time-to-mempool
    /// overlap. The first to be included wins the liquidation; the second
    /// will attempt the same borrower and revert harmlessly (costs gas only).
    ///
    /// Returns (strike_result, kill_result) — either can be None if that
    /// shot failed to send or get a receipt.
    pub async fn execute_parallel(
        &self,
        opp: &Opportunity,
        sim: &SimOutput,
        base_fee_wei: u128,
    ) -> (Option<ExecutionResult>, Option<ExecutionResult>) {
        let t = Instant::now();

        let gl = if sim.estimated_gas > 0 {
            ((sim.estimated_gas as u128 * 120 / 100).max(800_000)) as u64
        } else {
            800_000_u64
        };
        let gt_s = gas::compute_gas_tier(base_fee_wei, 1_000_000_000, gas::Tier::Strike, gas::Regime::Stable);
        let gt_k = gas::compute_gas_tier(base_fee_wei, 1_000_000_000, gas::Tier::Kill,   gas::Regime::Stable);

        if self.config.dry_run {
            info!(
                mode          = "DRY_RUN",
                borrower      = %opp.borrower,
                profit_usd    = sim.net_profit_usd,
                strike_gwei   = gt_s.max_fee_per_gas / 1_000_000_000,
                kill_gwei     = gt_k.max_fee_per_gas / 1_000_000_000,
                "DRY_RUN — parallel dual-shot NOT sent"
            );
            return (None, None);
        }

        // SOFT_LIVE parallel preview — print both shots, send neither
        if self.config.soft_live {
            let calldata = Bytes::from(
                IHuntLoanReceiver::requestFlashLiquidationCall {
                    debtAsset:       opp.debt_asset,
                    debtAmount:      U256::from(opp.debt_to_repay),
                    collateralAsset: opp.collateral_asset,
                    borrower:        opp.borrower,
                }
                .abi_encode(),
            );
            info!(
                mode              = "SOFT_LIVE",
                shot              = "STRIKE",
                to                = %self.config.huntloan_addr,
                max_fee_gwei      = gt_s.max_fee_per_gas / 1_000_000_000,
                max_priority_gwei = gt_s.max_priority_fee / 1_000_000_000,
                gas_limit         = gl,
                calldata          = %calldata,
                "SOFT_LIVE — dual-shot STRIKE preview (NOT broadcast)"
            );
            info!(
                mode              = "SOFT_LIVE",
                shot              = "KILL",
                to                = %self.config.huntloan_addr,
                max_fee_gwei      = gt_k.max_fee_per_gas / 1_000_000_000,
                max_priority_gwei = gt_k.max_priority_fee / 1_000_000_000,
                gas_limit         = gl,
                calldata          = %calldata,
                "SOFT_LIVE — dual-shot KILL preview (NOT broadcast)"
            );
            return (None, None);
        }

        let submit_url = self.config.private_rpc_http
            .as_deref()
            .unwrap_or(&self.config.rpc_http);
        let url = match submit_url.parse() {
            Ok(u)  => u,
            Err(e) => { warn!("Parallel shot: invalid RPC URL: {}", e); return (None, None); }
        };
        let provider = std::sync::Arc::new(
            ProviderBuilder::new().wallet(self.wallet.clone()).connect_http(url)
        );

        let nonce = match self.acquire_nonce(provider.as_ref()).await {
            Ok(n)  => n,
            Err(e) => { warn!("Parallel shot: nonce failed: {}", e); return (None, None); }
        };
        // Claim both nonce N and N+1 before any await
        *self.nonce.lock().await = Some(nonce + 2);

        info!(
            borrower     = %opp.borrower,
            nonce_strike = nonce,
            nonce_kill   = nonce + 1,
            "Parallel dual-shot: STRIKE + KILL"
        );

        let contract = IHuntLoanReceiver::new(self.config.huntloan_addr, provider.as_ref());

        // Send Shot 1 then Shot 2 before awaiting either receipt
        let sent1 = contract
            .requestFlashLiquidation(
                opp.debt_asset,
                U256::from(opp.debt_to_repay),
                opp.collateral_asset,
                opp.borrower,
            )
            .max_fee_per_gas(gt_s.max_fee_per_gas)
            .max_priority_fee_per_gas(gt_s.max_priority_fee)
            .gas(gl)
            .nonce(nonce)
            .send()
            .await;

        let sent2 = contract
            .requestFlashLiquidation(
                opp.debt_asset,
                U256::from(opp.debt_to_repay),
                opp.collateral_asset,
                opp.borrower,
            )
            .max_fee_per_gas(gt_k.max_fee_per_gas)
            .max_priority_fee_per_gas(gt_k.max_priority_fee)
            .gas(gl)
            .nonce(nonce + 1)
            .send()
            .await;

        // Concurrently wait for both receipts
        let (r1, r2) = tokio::join!(
            async {
                match sent1 {
                    Ok(pending) => {
                        let h = *pending.tx_hash();
                        pending.get_receipt().await.ok().map(|receipt| ExecutionResult {
                            tx_hash:         h,
                            block_number:    receipt.block_number.unwrap_or(0),
                            gas_used:        receipt.gas_used as u64,
                            send_latency_ms: t.elapsed().as_millis() as u64,
                        })
                    }
                    Err(e) => { warn!(shot = "STRIKE", error = %e, "Send failed"); None }
                }
            },
            async {
                match sent2 {
                    Ok(pending) => {
                        let h = *pending.tx_hash();
                        pending.get_receipt().await.ok().map(|receipt| ExecutionResult {
                            tx_hash:         h,
                            block_number:    receipt.block_number.unwrap_or(0),
                            gas_used:        receipt.gas_used as u64,
                            send_latency_ms: t.elapsed().as_millis() as u64,
                        })
                    }
                    Err(e) => { warn!(shot = "KILL", error = %e, "Send failed"); None }
                }
            }
        );

        (r1, r2)
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
    ) -> Result<ExecutionResult> {
        let t = Instant::now();
        let fees = self.compute_fees(opp.health_factor, base_fee_wei, sim.estimated_gas);

        if self.config.dry_run {
            info!(
                mode = "DRY_RUN",
                borrower = %opp.borrower,
                debt_usd = opp.debt_usd,
                estimated_profit = sim.net_profit_usd,
                max_fee_gwei = fees.max_fee_per_gas / 1_000_000_000,
                gas_limit = fees.gas_limit,
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
        // No transaction is sent. Use this to validate fees and calldata before
        // committing to a live broadcast.
        if self.config.soft_live {
            let provider = ProviderBuilder::new()
                .wallet(self.wallet.clone())
                .connect_http(self.config.rpc_http.parse().wrap_err("RPC_URL invalid")?);
            let nonce = self.acquire_nonce(&provider).await.unwrap_or(u64::MAX);

            let calldata = Bytes::from(
                IHuntLoanReceiver::requestFlashLiquidationCall {
                    debtAsset:        opp.debt_asset,
                    debtAmount:       U256::from(opp.debt_to_repay),
                    collateralAsset:  opp.collateral_asset,
                    borrower:         opp.borrower,
                }
                .abi_encode(),
            );

            info!(
                mode                 = "SOFT_LIVE",
                to                   = %self.config.huntloan_addr,
                chain_id             = self.config.chain_id,
                nonce                = nonce,
                max_fee_wei          = fees.max_fee_per_gas,
                max_fee_mgwei        = fees.max_fee_per_gas / 1_000_000,  // milli-gwei
                max_priority_wei     = fees.max_priority_fee,
                max_priority_mgwei   = fees.max_priority_fee / 1_000_000, // milli-gwei
                gas_limit            = fees.gas_limit,
                value_wei            = 0,
                calldata_bytes       = calldata.len(),
                calldata             = %calldata,
                borrower             = %opp.borrower,
                debt_to_repay        = opp.debt_to_repay,
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

        self.broadcast_with_retry(opp, &fees, t).await
    }

    // ── Broadcast + retry ────────────────────────────────────────────────────

    async fn broadcast_with_retry(
        &self,
        opp: &Opportunity,
        initial_fees: &TxFees,
        t: Instant,
    ) -> Result<ExecutionResult> {
        const MAX_ATTEMPTS: u8 = 3;
        const FEE_BUMP_PCT: u128 = 15; // +15% per retry

        // Use private RPC for submission when configured (MEV protection on Base)
        let submit_url = self.config.private_rpc_http
            .as_deref()
            .unwrap_or(&self.config.rpc_http);

        let provider = ProviderBuilder::new()
            .wallet(self.wallet.clone())
            .connect_http(submit_url.parse().wrap_err("RPC_URL invalid")?);

        // Micro-bankroll gate: refuse broadcast if wallet is below safety floor.
        // get_balance failure is non-fatal — we log and allow (balance unknown ≠ zero).
        let balance_wei = provider
            .get_balance(self.signer_address)
            .await
            .map(|b| b.to::<u128>())
            .unwrap_or(u128::MAX); // unknown balance → allow execution
        if balance_wei < self.config.min_wallet_eth_wei {
            bail!(
                "Wallet {:.6} ETH below safety floor {:.6} ETH — refusing broadcast to preserve capital",
                balance_wei as f64 / 1e18,
                self.config.min_wallet_eth_wei as f64 / 1e18,
            );
        }

        let nonce = self.acquire_nonce(&provider).await?;

        for attempt in 0_u8..MAX_ATTEMPTS {
            let bump = 100_u128 + FEE_BUMP_PCT * attempt as u128;
            let fees = TxFees {
                max_fee_per_gas:  initial_fees.max_fee_per_gas * bump / 100,
                max_priority_fee: initial_fees.max_priority_fee * bump / 100,
                gas_limit:        initial_fees.gas_limit,
            };

            info!(
                attempt = attempt + 1,
                borrower = %opp.borrower,
                max_fee_gwei = fees.max_fee_per_gas / 1_000_000_000,
                nonce = nonce,
                "Broadcasting liquidation tx"
            );

            let contract = IHuntLoanReceiver::new(self.config.huntloan_addr, &provider);

            let call = contract
                .requestFlashLiquidation(
                    opp.debt_asset,
                    U256::from(opp.debt_to_repay),
                    opp.collateral_asset,
                    opp.borrower,
                )
                .max_fee_per_gas(fees.max_fee_per_gas)
                .max_priority_fee_per_gas(fees.max_priority_fee)
                .gas(fees.gas_limit)
                .nonce(nonce);

            match call.send().await {
                Ok(pending) => {
                    let tx_hash = *pending.tx_hash();
                    info!(tx_hash = %tx_hash, "Tx submitted — waiting for receipt");

                    match pending.get_receipt().await {
                        Ok(receipt) => {
                            let block_number = receipt.block_number.unwrap_or(0);
                            let gas_used = receipt.gas_used as u64;
                            let send_ms = t.elapsed().as_millis() as u64;
                            self.confirm_nonce(nonce).await;
                            info!(
                                tx_hash = %tx_hash,
                                block = block_number,
                                gas_used = gas_used,
                                send_latency_ms = send_ms,
                                "Tx confirmed"
                            );
                            return Ok(ExecutionResult {
                                tx_hash,
                                block_number,
                                gas_used,
                                send_latency_ms: send_ms,
                            });
                        }
                        Err(e) => {
                            warn!(attempt = attempt + 1, error = %e, "Receipt wait failed");
                        }
                    }
                }
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("revert") || msg.contains("execution reverted") {
                        self.invalidate_nonce().await;
                        bail!("Tx reverted: {}", msg);
                    }
                    warn!(attempt = attempt + 1, error = %msg, "Send error — retrying");
                }
            }
        }

        self.invalidate_nonce().await;
        bail!("Execution failed after {} attempts", MAX_ATTEMPTS)
    }

    // ── EIP-1559 fee computation ──────────────────────────────────────────────

    fn compute_fees(&self, health_factor: f64, base_fee_wei: u128, est_gas: u64) -> TxFees {
        let tier = gas::select_tier(health_factor, 30.0);
        let regime = gas::Regime::Stable; // TODO: detect_regime from price feed
        let gas_tier = gas::compute_gas_tier(base_fee_wei, 1_000_000_000, tier, regime);

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

    // ── Nonce management ─────────────────────────────────────────────────────

    async fn acquire_nonce<P: Provider>(&self, provider: &P) -> Result<u64> {
        let mut guard = self.nonce.lock().await;
        if let Some(n) = *guard {
            *guard = Some(n + 1);
            return Ok(n);
        }
        let on_chain = provider
            .get_transaction_count(self.signer_address)
            .await
            .wrap_err("get_transaction_count failed")?;
        *guard = Some(on_chain + 1);
        Ok(on_chain)
    }

    async fn confirm_nonce(&self, sent_nonce: u64) {
        let mut guard = self.nonce.lock().await;
        if guard.map_or(true, |n| sent_nonce + 1 > n) {
            *guard = Some(sent_nonce + 1);
        }
    }

    async fn invalidate_nonce(&self) {
        *self.nonce.lock().await = None;
    }
}
