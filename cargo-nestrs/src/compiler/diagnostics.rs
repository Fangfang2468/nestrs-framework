//! 带 Nestrs 编号的源码诊断统一输出口。通过原生 rustc 同时交付终端和 Cargo JSON，
//! 不让经过语义认证的来源在最后一步退化成无位置的 stderr 字符串。
//! 宏语法错误、原生 Rust 错误和无法继续的工具链协议握手仍可使用各自的原生诊断。

use rustc_middle::ty::TyCtxt;
use rustc_span::{FileName, Span, SyntaxContext};
use std::hash::{DefaultHasher, Hash, Hasher};

/// 一次 Nestrs 错误及其 rustc 展示信息；cause 保留内部证据。
pub(crate) struct Diagnostic {
    /// 稳定的 Nestrs 诊断编号。
    pub code: &'static str,

    /// 面向用户的主要错误说明。
    pub message: String,

    /// 主诊断位置，允许无法恢复位置时使用 dummy span。
    pub primary: Span,

    /// 附着于业务源码范围的解释。
    pub labels: Vec<(Span, String)>,

    /// 不依赖单一源码位置的补充说明。
    pub notes: Vec<String>,

    /// 可操作的修复建议。
    pub help: Vec<String>,

    /// 完整内部证据，过长时另存诊断工件。
    pub cause: String,
}

impl Diagnostic {
    /// 创建主诊断，标签、建议和内部原因由调用者逐项补齐。
    pub fn new(code: &'static str, message: String, primary: Span) -> Self {
        Self {
            code,
            message,
            primary,
            labels: vec![],
            notes: vec![],
            help: vec![],
            cause: String::new(),
        }
    }
}

/// 来源 marker 已经过身份/签名认证。只对展示清除展开上下文；真实 Span 的字节
/// 范围仍指向用户 token，不影响语义解析或宏卫生。不把虚拟生成文件伪装成业务文件。
pub(crate) fn source_span(tcx: TyCtxt<'_>, span: Span) -> Span {
    if span.is_dummy() {
        return span;
    }
    let file = tcx.sess.source_map().lookup_source_file(span.lo());
    // 外部 metadata 的源码仅用于诊断展示，不是本轮第二阶段的新编译输入。
    // SourceFile 会以 metadata 中的 src_hash 校验内容；缺失/已变化文件不能伪造
    // 片段。不要放宽 CheckedLoader 对真正编译输入的两阶段快照检查。
    if file.is_imported() {
        file.add_external_src(|| match &file.name {
            FileName::Real(name) => name
                .local_path()
                .and_then(|path| std::fs::read_to_string(path).ok()),
            _ => None,
        });
    }
    if matches!(&file.name, FileName::Real(_)) {
        span.with_ctxt(SyntaxContext::root())
    } else {
        let callsite = span.source_callsite();
        if callsite == span {
            rustc_span::DUMMY_SP
        } else {
            source_span(tcx, callsite)
        }
    }
}

/// 按用户源码位置稳定排序并输出全部诊断，最后通过 rustc 的受控错误出口结束编译。
pub(crate) fn emit(tcx: TyCtxt<'_>, mut diagnostics: Vec<Diagnostic>) -> ! {
    diagnostics.sort_by_key(|d| {
        let span = source_span(tcx, d.primary);
        if span.is_dummy() {
            (String::new(), 0, 0, d.code, d.message.clone())
        } else {
            let loc = tcx.sess.source_map().lookup_char_pos(span.lo());
            (
                loc.file.name.prefer_local_unconditionally().to_string(),
                loc.line,
                loc.col.0,
                d.code,
                d.message.clone(),
            )
        }
    });
    for report in diagnostics {
        let primary = source_span(tcx, report.primary);
        let mut error = tcx
            .dcx()
            .struct_span_err(primary, format!("[{}] {}", report.code, report.message));
        let mut unavailable = std::collections::BTreeSet::new();
        let mut observe_source = |span: Span| {
            if !span.is_dummy() && tcx.sess.source_map().span_to_snippet(span).is_err() {
                let file = tcx.sess.source_map().lookup_source_file(span.lo());
                unavailable.insert(file.name.prefer_local_unconditionally().to_string());
            }
        };
        observe_source(primary);
        for (span, label) in report.labels {
            let span = source_span(tcx, span);
            observe_source(span);
            if !span.is_dummy() {
                error.span_label(span, label);
            }
        }
        for file in unavailable {
            error.note(format!("未能加载 `{file}` 的源码片段（文件不可用或与编译 metadata 不一致）；以上位置来自原编译记录。"));
        }
        for note in report.notes {
            error.note(note);
        }
        if primary.is_dummy() {
            error.note("未能恢复精确的用户源码位置；已知声明与内部原因见 cause。");
        }
        for help in report.help {
            error.help(help);
        }
        // 长路径和大候选集不淹没终端；完整依据写入该编译单元的诊断工件，且明确
        // 报告写入失败，不能声称被省略的信息仍可取回。
        let cause = if report.cause.chars().count() > 1800 {
            let mut hash = DefaultHasher::new();
            report.cause.hash(&mut hash);
            let path = tcx
                .output_filenames(())
                .temp_path_for_diagnostic(&format!("nestrs-cause-{:x}.txt", hash.finish()));
            let prefix: String = report.cause.chars().take(600).collect();
            match std::fs::write(&path, &report.cause) {
                Ok(()) => format!("{prefix}\n… 内部详情已省略；完整 cause：{}", path.display()),
                Err(failure) => {
                    format!("{prefix}\n… 内部详情已截断，写入完整 cause 失败：{failure}")
                }
            }
        } else {
            report.cause
        };
        error.note(format!("cause: {cause}"));
        error.emit();
    }
    // 由 rustc 的受控错误退出结束当前编译，避免 MIR/after_analysis/CLI 二次包装。
    tcx.dcx().abort_if_errors();
    unreachable!("至少一条诊断已发出")
}

/// 将内部 metadata 或执行协议不一致报告为工具错误，避免误导用户修改私有 ABI。
pub(crate) fn internal(tcx: TyCtxt<'_>, cause: String) -> ! {
    let mut diagnostic = Diagnostic::new(
        "NESTRS-TOOL001",
        "工具内部错误：依赖描述或计划协议不一致".into(),
        rustc_span::DUMMY_SP,
    );
    diagnostic.help.push(
        "请保留此诊断及工具版本用于排查；此错误不表示业务代码需要修改内部槽位或 metadata。".into(),
    );
    diagnostic.cause = cause;
    emit(tcx, vec![diagnostic])
}
