// PM2 ecosystem config for HuntLoan liquidation engine.
// Usage: pm2 start ecosystem.config.cjs
module.exports = {
  apps: [
    {
      name: "huntloan",
      script: "./target/release/huntloan",
      cwd: "/root/huntloan",

      // Graceful shutdown — matches SIGTERM handler in engine.rs
      kill_timeout: 10000, // 10s grace period before SIGKILL
      listen_timeout: 5000,

      // Restart policy
      autorestart: true,
      max_restarts: 10,
      min_uptime: "30s",
      restart_delay: 5000, // 5s between restarts

      // Memory guard
      max_memory_restart: "512M",

      // Logging
      log_date_format: "YYYY-MM-DD HH:mm:ss Z",
      error_file: "/root/.pm2/logs/huntloan-error.log",
      out_file: "/root/.pm2/logs/huntloan-out.log",
      merge_logs: true,
      log_type: "json",

      // Environment
      env: {
        RUST_LOG: "huntloan=info",
        RUST_BACKTRACE: "1",
      },
    },
  ],
};
