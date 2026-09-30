//! 结账应用的唯一 crate 入口；业务模块属于这个 binary，不发布应用库。
mod application;
mod checkout;
mod cli;
mod config;
mod domain;
mod infrastructure;
mod observe;

#[cfg(test)]
mod tests;

use std::process::ExitCode;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> ExitCode {
    let invocation = match cli::parse() {
        Ok(Some(invocation)) => invocation,
        Ok(None) => return ExitCode::SUCCESS,
        Err(error) => {
            let _ = error.print();
            return ExitCode::from(error.exit_code() as u8);
        }
    };
    let sample = invocation.command.is_sample();
    match application::run(invocation.options, invocation.command.requests()).await {
        Ok(report) => {
            cli::print_report(&report);
            if sample
                || report
                    .responses
                    .iter()
                    .all(|response| response.result.is_ok())
            {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("结账应用执行失败：{error}");
            ExitCode::FAILURE
        }
    }
}
