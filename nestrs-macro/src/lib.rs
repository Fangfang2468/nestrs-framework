mod injection;

use zyn::{meta::Args, syn::{self, spanned::Spanned}, zyn};

use crate::injection::injectable::config::InjectableConfig;






#[zyn::attribute]
pub fn injectable(
    #[zyn(input)] item: syn::ItemStruct,    // 被标注的项，自动提取
    args: Args,                             // #[injectable(...)] 里的原始参数
) -> zyn::TokenStream {
    // 1) 严格校验参数形态：只允许命名参数，且不允许重复
    let mut seen: Vec<String> = Vec::new();
    
    for arg in args.iter() {
        let Some(name) = arg.name() else {
            bail!(
                "`#[injectable]` 参数填写格式错误";
                span = arg.span()
            );
        };

        let name = name.to_string();
        if seen.contains(&name) {
            bail!("`#[injectable]` 参数 `{name}` 重复声明"; span = arg.span());
        }
        seen.push(name);
    }

    // 2) 强类型解析参数；失败时直接发射诊断
    let config = match InjectableConfig::from_args(&args) {
        Ok(cfg) => cfg,
        Err(diag) => return diag.emit().into(),
    };

    println!("[调试]：解析 `#[injectable]` 参数成功，配置为：\n{config:#?}");

    // 结构体名称
    let struct_name = item.ident.clone();


    zyn! {
        {{ item }}
    }
}
