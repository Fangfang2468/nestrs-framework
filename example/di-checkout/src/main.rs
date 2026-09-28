use std::{num::NonZeroUsize, process::ExitCode};

use nestrs_core::InitializationMode;
use nestrs_di_example::demo::{self, DemoOptions};

const HELP: &str = "Nestrs DI 电商下单示例

运行：cargo nestrs run -p nestrs-di-example -- [选项]

  --eager                  build 时预热全部 Singleton（默认 Lazy）
  --warm-up-scopes         每笔请求先预热当前 scope 的全部 Scoped 服务
  --max-concurrency N      全 root 的构造并发上限，N 必须大于 0（默认 4）
  -h, --help               显示帮助

故障演示：
  NESTRS_EXAMPLE_FAIL_PAYMENT=1 cargo nestrs run -p nestrs-di-example

依赖图：cargo nestrs graph -p nestrs-di-example

所有数据、支付和连接均在本地模拟，无需数据库、网络服务或账户凭证。";

fn options() -> Result<Option<DemoOptions>, String> {
    let mut options = DemoOptions::default();
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--eager" => options.initialization = InitializationMode::Eager,
            "--warm-up-scopes" => options.warm_up_scopes = true,
            "--max-concurrency" => {
                options.max_concurrent_activations = arguments
                    .next()
                    .ok_or("--max-concurrency 需要一个正整数")?
                    .parse::<NonZeroUsize>()
                    .map_err(|_| "--max-concurrency 必须是大于 0 的整数")?;
            }
            "--help" | "-h" => return Ok(None),
            _ => return Err(format!("未知选项：{argument}，使用 --help 查看用法")),
        }
    }
    Ok(Some(options))
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> ExitCode {
    let options = match options() {
        Ok(Some(options)) => options,
        Ok(None) => {
            println!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!("参数错误：{error}");
            return ExitCode::from(2);
        }
    };
    match demo::run(options).await {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("示例执行失败：{error}");
            ExitCode::FAILURE
        }
    }
}
