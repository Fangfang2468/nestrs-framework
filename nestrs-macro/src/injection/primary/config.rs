use zyn::{Args, FromArg};

use crate::injection::primary::attrs::PrimaryInterface;


#[derive(Clone, Debug, Default)]
pub struct PrimaryConfig {
    /// 手动指定的Trait名称
    pub trait_names: PrimaryInterface,
}

impl PrimaryConfig {
    pub fn from_args(args: &Args) -> zyn::Result<Self> {
        let primary_args = zyn::Arg::List(zyn::format_ident!("primary"), args.clone());

        Ok(Self {
            trait_names: PrimaryInterface::from_arg(&primary_args)?,
        })
    }
}
