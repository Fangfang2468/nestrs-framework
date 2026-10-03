//! 在标准宏展开之后、HIR 降低之前协调显式构造函数与服务字段。
//!
//! 属性宏分别生成 struct 候选构造路径和 impl 内的真实 typed adapter。两者之间
//! 不使用进程全局声明收集，也不重新扫描源码；本模块只查看 rustc 已经完成 cfg、
//! 外部模块和宏展开的 AST，以及该 AST 对应的真实名称解析结果。它按 struct DefId
//! 选择候选路径，并把保存注入令牌的字段包成与构造参数一致的存储类型。
//!
//! 改写发生在 `resolver_for_lowering_raw` 返回之前。原查询结果的 AST/resolver 被
//! 一次性取走，修改后装入新的 Steal；不修改已公布的 query cache，不跳过后续 HIR、
//! 类型、借用或隐私检查。新增的 wrapper 节点有独立 NodeId，内层业务类型保留字段
//! 原有 AST 和解析，因此不同 impl 泛型名称不会串用类型参数身份。

use crate::internal_access;
use crate::protocol::constructor::{
    ACTIVATE, DEPENDENCIES, METADATA as CONSTRUCTOR_METADATA, Metadata,
};
use rustc_ast::{
    ast,
    mut_visit::{self, MutVisitor},
    visit::{self, Visitor},
};
use rustc_data_structures::steal::Steal;
use rustc_hir::{
    def::{DefKind, PartialRes, Res},
    def_id::DefId,
};
use rustc_middle::{
    ty::{self, TyCtxt},
    util::Providers,
};
use rustc_span::Span;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

#[path = "constructor_body.rs"]
mod body;

/// 原生名称解析到 AST lowering 的查询签名；包装器必须整体保留三份结果。
type LoweringQuery = for<'tcx> fn(
    TyCtxt<'tcx>,
    (),
) -> (
    &'tcx Steal<ty::ResolverAstLowering<'tcx>>,
    &'tcx Steal<ast::Crate>,
    &'tcx ty::ResolverGlobalCtxt,
);

/// 仅初始化一次的原生 lowering 查询，用于避免包装器递归调用自己。
static ORIGINAL: OnceLock<LoweringQuery> = OnceLock::new();

/// 本轮是否捕获基于物理源码的 IDE 模型。
static CAPTURE_IDE: AtomicBool = AtomicBool::new(false);

/// 仅 discovery 使用原始物理源码坐标；最终 overlay 阶段不得覆盖该模型。
pub fn capture_ide(enabled: bool) {
    CAPTURE_IDE.store(enabled, Ordering::Relaxed);
}

/// 保存原始 lowering 查询并安装 AST 协调钩子；后续原生类型与借用检查继续执行。
pub fn provide(providers: &mut Providers) {
    let _ = ORIGINAL.set(providers.queries.resolver_for_lowering_raw);
    providers.queries.resolver_for_lowering_raw = lower_constructors;
}

/// 某个真实服务类型选中的显式构造路径及字段来源。
struct Constructor {
    /// 显式构造声明的位置，供冲突和来源错误诊断。
    span: Span,

    /// 生成 adapter 使用的构造输入绑定名称。
    input: String,

    /// 业务字段到输入槽位及参数存储类型的映射。
    fields: BTreeMap<String, (usize, Box<ast::Ty>)>,

    /// 调用用户构造函数的已认证 helper。
    activate: Helper,

    /// 提供参数依赖描述的已认证 helper。
    dependencies: Helper,
}

/// 构造 helper 的解析身份；名称只用于重写 AST 引用。
#[derive(Clone, Copy)]
struct Helper {
    /// 保留宏卫生上下文的 helper 标识符。
    ident: rustc_span::Ident,

    /// 名称解析确认的真实定义。
    definition: DefId,
}

/// impl -> 实际 struct 的映射只依赖名称解析，不能在这里调用 type_of 等 HIR 查询。
/// 否则 query 会在自己的结果尚未构造完毕时重入，形成循环。
struct Declarations<'a, 'tcx> {
    /// 已完成标准名称解析、尚未降低为 HIR 的结果。
    resolver: &'a ty::ResolverAstLowering<'tcx>,

    /// inherent impl 到其实际服务 struct 的映射。
    impls: HashMap<DefId, DefId>,

    /// 服务 struct 的真实定义及原始标识符。
    structs: HashMap<DefId, rustc_span::Ident>,

    /// 按声明顺序记录泛型参数种类，以验证 impl 映射。
    struct_parameters: HashMap<DefId, Vec<ParameterKind>>,

    /// 仅 IDE 捕获阶段收集潜在名称冲突。
    capture_identifiers: bool,

    /// 已展开源码中出现的名称，供 IDE adapter 避让。
    identifiers: HashSet<String>,
}

impl<'ast> Visitor<'ast> for Declarations<'_, '_> {
    /// 收集 IDE adapter 需要避让的已展开标识符，包括真实方法调用名称。
    fn visit_ident(&mut self, ident: &'ast rustc_span::Ident) {
        // IDE 的 inherent adapter 也可能遮蔽 trait 默认成员，或影响通过 Deref
        // 查找的业务方法。标准展开后的完整标识符目录同时包括本地声明及真实
        // 路径/方法调用，覆盖上游 blanket impl 和隐式 Deref，且无需在 HIR
        // 尚未形成时触发 trait/type 查询。保守多排除一个名称不会改变业务语义。
        if self.capture_identifiers {
            self.identifiers.insert(ident.name.to_string());
        }
    }

    /// 记录 struct 泛型形状及 inherent impl 的真实 Self 解析关系。
    fn visit_item(&mut self, item: &'ast ast::Item) {
        if let Some(owner) = self.resolver.owners.get(&item.id) {
            match &item.kind {
                ast::ItemKind::Struct(_, generics, _) => {
                    self.struct_parameters.insert(
                        owner.def_id.to_def_id(),
                        generics
                            .params
                            .iter()
                            .map(|parameter| parameter_kind(&parameter.kind))
                            .collect(),
                    );
                    self.structs.insert(
                        owner.def_id.to_def_id(),
                        item.kind.ident().expect("struct identifier"),
                    );
                }
                ast::ItemKind::Impl(implementation) => {
                    if let Some(resolution) = self
                        .resolver
                        .partial_res_map
                        .get(&implementation.self_ty.id)
                        && let Res::Def(DefKind::Struct, service) = resolution.base_res()
                    {
                        self.impls.insert(owner.def_id.to_def_id(), service);
                    }
                }
                _ => {}
            }
        }
        visit::walk_item(self, item);
    }
}

/// 从已认证构造元数据收集每个服务的字段与 adapter 信息。
struct Collect<'a, 'tcx> {
    /// 本轮编译上下文与诊断入口。
    tcx: TyCtxt<'tcx>,

    /// AST 节点对应的真实名称解析结果。
    resolver: &'a ty::ResolverAstLowering<'tcx>,

    /// 已建立的 impl、struct 与泛型参数目录。
    declarations: &'a Declarations<'a, 'tcx>,

    /// 按服务 DefId 保存的显式构造路径。
    constructors: HashMap<DefId, Constructor>,
}

impl<'ast> Visitor<'ast> for Collect<'_, '_> {
    /// 读取已认证的 constructor 元数据，验证参数与成功字段来源。
    fn visit_item(&mut self, item: &'ast ast::Item) {
        if let ast::ItemKind::Impl(implementation) = &item.kind {
            for associated in &implementation.items {
                let ast::AssocItemKind::Const(constant) = &associated.kind else {
                    continue;
                };
                if constant.ident.as_str() != CONSTRUCTOR_METADATA
                    || !internal_access::trusted_span(self.tcx, constant.ident.span)
                {
                    continue;
                }
                let ast::ConstItemRhsKind::Body {
                    rhs: Some(expression),
                } = &constant.rhs_kind
                else {
                    continue;
                };
                let ast::ExprKind::Lit(literal) = expression.kind else {
                    continue;
                };
                let Ok(ast::LitKind::Str(json, _)) = ast::LitKind::from_token_lit(literal) else {
                    continue;
                };
                let metadata: Metadata = match serde_json::from_str(json.as_str()) {
                    Ok(value) => value,
                    Err(error) => {
                        self.tcx.dcx().span_err(
                            associated.span,
                            format!("constructor 声明元数据无效：{error}"),
                        );
                        continue;
                    }
                };
                let service = self
                    .resolver
                    .owners
                    .get(&item.id)
                    .and_then(|owner| self.declarations.impls.get(&owner.def_id.to_def_id()))
                    .copied();
                let Some(service) = service.filter(|service| {
                    implementation.of_trait.is_none()
                        && self.declarations.struct_parameters.contains_key(service)
                }) else {
                    self.tcx.dcx().span_err(associated.span, "#[constructor] 必须位于本 crate 服务结构体的 inherent impl 中；不能用于 trait impl 或无法确定服务身份的类型别名");
                    continue;
                };
                let Some(function) =
                    implementation
                        .items
                        .iter()
                        .find_map(|candidate| match &candidate.kind {
                            ast::AssocItemKind::Fn(function)
                                if function.ident.as_str() == unraw(&metadata.method) =>
                            {
                                Some(function)
                            }
                            _ => None,
                        })
                else {
                    self.tcx
                        .dcx()
                        .span_err(associated.span, "constructor 元数据没有对应的构造函数");
                    continue;
                };
                if !generic_constructor_covers_service(
                    self.resolver,
                    item.id,
                    implementation,
                    &self.declarations.struct_parameters[&service],
                ) {
                    self.tcx.dcx().span_err(function.ident.span, "#[constructor] 的 impl Self 必须覆盖服务的全部泛型实例：按服务参数位置使用 impl 自身互不重复的类型、生命周期或 const 参数；不支持具体类型、嵌套类型或重复参数的专门化构造 impl");
                    continue;
                }
                let mut fields = BTreeMap::new();
                // 不按业务成员的拼写连接 adapter。两个属性宏的 def-site 上下文
                // 不同；保存本次 constructor 真正生成的 DefId，选中候选时完成
                // 工具生成调用的解析，业务类型和参数 token 的上下文不变。
                let helper = |name| {
                    implementation.items.iter().find_map(|candidate| {
                        let ast::AssocItemKind::Fn(function) = &candidate.kind else {
                            return None;
                        };
                        (function.ident.as_str() == name
                            && internal_access::trusted_span(self.tcx, function.ident.span))
                        .then(|| {
                            self.resolver.owners.get(&candidate.id).map(|owner| Helper {
                                ident: function.ident,
                                definition: owner.def_id.to_def_id(),
                            })
                        })
                        .flatten()
                    })
                };
                let (Some(activate), Some(dependencies)) = (helper(ACTIVATE), helper(DEPENDENCIES))
                else {
                    self.tcx
                        .dcx()
                        .span_err(associated.span, "constructor 缺少对应的内部执行适配器");
                    continue;
                };
                match body::fields(
                    self.tcx,
                    self.resolver,
                    &self.declarations.impls,
                    service,
                    function,
                    metadata.result,
                ) {
                    Ok(mapping) => {
                        for (name, slot) in mapping {
                            fields.insert(name, (slot, function.sig.decl.inputs[slot].ty.clone()));
                        }
                    }
                    Err((span, message)) => {
                        self.tcx.dcx().span_err(span, message);
                    }
                }
                if self
                    .constructors
                    .insert(
                        service,
                        Constructor {
                            span: function.ident.span,
                            input: metadata.input,
                            fields,
                            activate,
                            dependencies,
                        },
                    )
                    .is_some()
                {
                    self.tcx.dcx().span_err(
                        function.ident.span,
                        "同一服务只能声明一个 #[constructor] 构造入口",
                    );
                }
            }
        }
        visit::walk_item(self, item);
    }
}

/// 比较协议名称时去除原始标识符前缀，不修改业务 token 的卫生来源。
fn unraw(name: &str) -> &str {
    name.strip_prefix("r#").unwrap_or(name)
}

/// 泛型参数的语法种类，用于校验 struct 与 impl 的位置对应。
#[derive(Clone, Copy, PartialEq, Eq)]
enum ParameterKind {
    /// 生命周期参数。
    Lifetime,

    /// 类型参数。
    Type,

    /// const 参数。
    Const,
}

/// 提取泛型参数种类，供 struct 与 impl 的位置映射校验。
fn parameter_kind(kind: &ast::GenericParamKind) -> ParameterKind {
    match kind {
        ast::GenericParamKind::Lifetime => ParameterKind::Lifetime,
        ast::GenericParamKind::Type { .. } => ParameterKind::Type,
        ast::GenericParamKind::Const { .. } => ParameterKind::Const,
    }
}

/// 连接真实 AssocFn 前必须保证其 impl Self 对完整服务类型通用。原生关联方法
/// probe 原本会做这一步，直接完成 Res 不能跳过此前提后让 typeck 假定它已成立。
/// 这里只接受已解析的参数身份一一覆盖，允许改名、重排与 const 的无运算括号；
/// 具体/嵌套/重复实参不是通用 constructor。剩余业务 bounds 继续由 rustc 检查。
fn generic_constructor_covers_service(
    resolver: &ty::ResolverAstLowering<'_>,
    owner: ast::NodeId,
    implementation: &ast::Impl,
    parameters: &[ParameterKind],
) -> bool {
    if parameters.is_empty() {
        return true;
    }
    let Some(owner) = resolver.owners.get(&owner) else {
        return false;
    };
    let declared: HashMap<_, _> = implementation
        .generics
        .params
        .iter()
        .filter_map(|parameter| {
            owner
                .node_id_to_def_id
                .get(&parameter.id)
                .map(|definition| (definition.to_def_id(), parameter_kind(&parameter.kind)))
        })
        .collect();
    let ast::TyKind::Path(None, path) = &implementation.self_ty.kind else {
        return false;
    };
    let Some(ast::GenericArgs::AngleBracketed(arguments)) = path
        .segments
        .last()
        .and_then(|segment| segment.args.as_deref())
    else {
        return false;
    };
    if arguments.args.len() != parameters.len() {
        return false;
    }
    let direct_parameter = |id| match resolver.partial_res_map.get(&id)?.full_res()? {
        Res::Def(DefKind::TyParam | DefKind::ConstParam, definition) => Some(definition),
        _ => None,
    };

    /// 仅接受直接类型参数引用及其外层括号，返回原始节点身份。
    fn type_parameter(value: &ast::Ty) -> Option<ast::NodeId> {
        match &value.kind {
            ast::TyKind::Path(None, path)
                if path.segments.len() == 1 && path.segments[0].args.is_none() =>
            {
                Some(value.id)
            }
            ast::TyKind::Paren(inner) => type_parameter(inner),
            _ => None,
        }
    }

    /// 识别无运算的 const 参数引用，允许括号或单表达式块包裹。
    fn const_parameter(value: &ast::Expr) -> Option<ast::NodeId> {
        match &value.kind {
            ast::ExprKind::Path(None, path)
                if path.segments.len() == 1 && path.segments[0].args.is_none() =>
            {
                Some(value.id)
            }
            ast::ExprKind::Paren(inner) => const_parameter(inner),
            ast::ExprKind::Block(block, None) if block.stmts.len() == 1 => {
                if let ast::StmtKind::Expr(inner) = &block.stmts[0].kind {
                    const_parameter(inner)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
    let mut seen = HashSet::new();
    for (argument, expected) in arguments.args.iter().zip(parameters) {
        let definition = match argument {
            ast::AngleBracketedArg::Arg(ast::GenericArg::Lifetime(lifetime)) => {
                match owner.get_lifetime_res(lifetime.id) {
                    Some(rustc_hir::def::LifetimeRes::Param { param, .. }) => {
                        Some(param.to_def_id())
                    }
                    _ => None,
                }
            }
            ast::AngleBracketedArg::Arg(ast::GenericArg::Type(value)) => {
                type_parameter(value).and_then(direct_parameter)
            }
            ast::AngleBracketedArg::Arg(ast::GenericArg::Const(value)) => {
                const_parameter(&value.value).and_then(direct_parameter)
            }
            _ => None,
        };
        let Some(definition) = definition else {
            return false;
        };
        if declared.get(&definition) != Some(expected) || !seen.insert(definition) {
            return false;
        }
    }
    seen.len() == declared.len()
}

/// 从已完成的名称解析识别服务 struct 或 impl Self，避免触发尚不可用的 HIR 查询。
fn service_resolution(
    resolver: &ty::ResolverAstLowering<'_>,
    impls: &HashMap<DefId, DefId>,
    node: ast::NodeId,
) -> Option<DefId> {
    match resolver.partial_res_map.get(&node)?.base_res() {
        Res::Def(DefKind::Struct, service) => Some(service),
        Res::SelfTyAlias { alias_to, .. } | Res::SelfCtor(alias_to) => {
            impls.get(&alias_to).copied()
        }
        _ => None,
    }
}

/// 只有由工具桥接生成的候选 if 才可折叠。业务的 if false、同名函数和伪造常量
/// 不会因此获得编译器权限，也不会按源码字符串误判服务的实际类型。
fn candidate(
    tcx: TyCtxt<'_>,
    expression: &ast::Expr,
    resolver: &ty::ResolverAstLowering<'_>,
    impls: &HashMap<DefId, DefId>,
) -> Option<(DefId, bool)> {
    let ast::ExprKind::If(condition, branch, Some(_)) = &expression.kind else {
        return None;
    };
    if !internal_access::trusted_span(tcx, expression.span) {
        return None;
    }
    let ast::ExprKind::Lit(literal) = condition.kind else {
        return None;
    };
    if !matches!(
        ast::LitKind::from_token_lit(literal),
        Ok(ast::LitKind::Bool(false))
    ) {
        return None;
    }
    let statement = branch.stmts.last()?;
    let (ast::StmtKind::Expr(call) | ast::StmtKind::Semi(call)) = &statement.kind else {
        return None;
    };
    let ast::ExprKind::Call(function, _) = &call.kind else {
        return None;
    };
    let ast::ExprKind::Path(_, path) = &function.kind else {
        return None;
    };
    let name = path.segments.last()?.ident.as_str();
    if !matches!(name, ACTIVATE | DEPENDENCIES) {
        return None;
    }
    Some((
        service_resolution(resolver, impls, function.id)?,
        name == DEPENDENCIES,
    ))
}

/// 把选中的显式构造路径写回 AST，并保持节点解析身份。
struct Rewrite<'a, 'tcx> {
    /// 编译会话及错误报告入口。
    tcx: TyCtxt<'tcx>,

    /// 需要为新增 AST 节点同步更新的解析结果。
    resolver: &'a mut ty::ResolverAstLowering<'tcx>,

    /// 用于识别 Self 或关联路径的真实 impl 映射。
    impls: &'a HashMap<DefId, DefId>,

    /// 本阶段可选择的显式构造路径。
    constructors: &'a HashMap<DefId, Constructor>,

    /// 已经选中过构造路径的服务，防止重复改写。
    selected: HashSet<DefId>,

    /// 服务到构造输入名称的映射，供字段包装使用。
    inputs: HashMap<DefId, String>,
}

impl MutVisitor for Rewrite<'_, '_> {
    /// 按参数来源改写真实服务字段的存储包装，保留业务字段的类型节点。
    fn visit_item(&mut self, item: &mut ast::Item) {
        if let ast::ItemKind::Struct(_, _, data) = &mut item.kind
            && let Some(owner) = self.resolver.owners.get(&item.id)
            && let Some(constructor) = self.constructors.get(&owner.def_id.to_def_id())
        {
            let mut pending: HashSet<_> = constructor.fields.keys().cloned().collect();
            let fields: &mut [ast::FieldDef] = match data {
                ast::VariantData::Struct { fields, .. } | ast::VariantData::Tuple(fields, _) => {
                    fields
                }
                ast::VariantData::Unit(_) => &mut [],
            };
            for field in fields {
                let Some(name) = field.ident.map(|ident| ident.name.to_string()) else {
                    continue;
                };
                // syn 的源码拼写保留 r#，rustc 的 Symbol 只保存真实名称。
                // 匹配时兼容两种拼写，保留原 key 供 IDE 和诊断重放。
                let mapping = constructor
                    .fields
                    .get_key_value(&name)
                    .or_else(|| constructor.fields.get_key_value(&format!("r#{name}")));
                let Some((original_name, (_, template))) = mapping else {
                    continue;
                };
                pending.remove(original_name);
                if let Err(reason) = rewrite_field(self.resolver, &mut field.ty, template) {
                    self.tcx.dcx().span_err(field.span, reason);
                }
            }
            for missing in pending {
                self.tcx.dcx().span_err(constructor.span, format!("constructor 中保存依赖的字段 `{missing}` 不属于该服务；必须返回当前服务自身的结构体实例"));
            }
        }
        mut_visit::walk_item(self, item);
    }

    /// 根据真实服务身份选择显式构造或自动字段分支，并连接已认证 helper。
    fn visit_expr(&mut self, expression: &mut ast::Expr) {
        if let Some((service, dependencies)) =
            candidate(self.tcx, expression, self.resolver, self.impls)
        {
            self.selected.insert(service);
            let use_constructor = self.constructors.contains_key(&service);
            if dependencies && let ast::ExprKind::If(_, _, Some(otherwise)) = &expression.kind {
                let mut marker = FieldMode::default();
                marker.visit_expr(otherwise);
                if let Some(input) = marker.input {
                    self.inputs.insert(service, input);
                }
                if marker.mixed && use_constructor {
                    self.tcx.dcx().span_err(self.constructors[&service].span, "显式 constructor 模式不能同时使用字段 #[inject]、#[lazy] 或 #[value]；依赖请声明在构造参数上，字段由构造函数完整初始化");
                }
            }
            if let ast::ExprKind::If(_, branch, Some(otherwise)) = &mut expression.kind {
                expression.kind = if use_constructor {
                    let constructor = &self.constructors[&service];
                    connect_helper(
                        self.resolver,
                        branch,
                        if dependencies {
                            constructor.dependencies
                        } else {
                            constructor.activate
                        },
                    );
                    ast::ExprKind::Block(branch.clone(), None)
                } else {
                    otherwise.kind.clone()
                };
            }
        }
        mut_visit::walk_expr(self, expression);
    }
}

/// candidate 已认证分支及真实服务身份。以真实 DefId 完成内部关联调用的解析；
/// def-site 标识符仍隔离业务同名成员，不通过改业务 impl 的上下文连接两个宏。
/// 原路径前缀变为 QSelf，保留原有泛型参数和解析结果，继续由原生 typeck 检查
/// 目标 impl 的 Self、参数、返回类型与约束；不向业务表达式传播生成访问权限。
fn connect_helper(
    resolver: &mut ty::ResolverAstLowering<'_>,
    branch: &mut ast::Block,
    helper: Helper,
) {
    let Some(statement) = branch.stmts.last_mut() else {
        return;
    };
    let (ast::StmtKind::Expr(call) | ast::StmtKind::Semi(call)) = &mut statement.kind else {
        return;
    };
    let ast::ExprKind::Call(function, _) = &mut call.kind else {
        return;
    };
    let ast::ExprKind::Path(qself, path) = &mut function.kind else {
        return;
    };
    let Some(original) = resolver.partial_res_map.get(&function.id).copied() else {
        return;
    };
    if qself.is_none() && path.segments.len() > 1 {
        let mut prefix = path.clone();
        prefix.segments.pop();
        let id = fresh_node(resolver, function.id);
        resolver
            .partial_res_map
            .insert(id, PartialRes::new(original.base_res()));
        *qself = Some(Box::new(ast::QSelf {
            ty: Box::new(ast::Ty {
                id,
                kind: ast::TyKind::Path(None, prefix),
                span: path.span,
                tokens: None,
            }),
            path_span: path.span.shrink_to_lo(),
            position: 0,
        }));
        let last = path.segments.pop().expect("nonempty helper path");
        path.segments.clear();
        path.segments.push(last);
    }
    if let Some(segment) = path.segments.last_mut() {
        segment.ident.span = segment.ident.span.with_ctxt(helper.ident.span.ctxt());
        let resolution = PartialRes::new(Res::Def(DefKind::AssocFn, helper.definition));
        resolver.partial_res_map.insert(function.id, resolution);
        resolver.partial_res_map.insert(segment.id, resolution);
    }
}

/// 自动字段构造标记的扫描结果，用于拒绝与显式构造混用。
#[derive(Default)]
struct FieldMode {
    /// 是否发现显式构造不允许共存的字段注入配置。
    mixed: bool,

    /// 生成默认构造路径使用的输入绑定名称。
    input: Option<String>,
}

impl<'ast> Visitor<'ast> for FieldMode {
    /// 读取生成的字段模式标记，以判断显式构造与自动字段配置是否混用。
    fn visit_local(&mut self, local: &'ast ast::Local) {
        if let ast::PatKind::Ident(_, ident, _) = &local.pat.kind
            && ident.as_str() == "__nestrs_constructor_field_mode"
            && let ast::LocalKind::Init(expression) = &local.kind
            && let ast::ExprKind::Lit(literal) = expression.kind
            && matches!(
                ast::LitKind::from_token_lit(literal),
                Ok(ast::LitKind::Bool(true))
            )
        {
            self.mixed = true;
        }
        if let ast::PatKind::Ident(_, ident, _) = &local.pat.kind
            && ident.as_str() == "__nestrs_constructor_input"
            && let ast::LocalKind::Init(expression) = &local.kind
            && let ast::ExprKind::Lit(literal) = expression.kind
            && let Ok(ast::LitKind::Str(input, _)) = ast::LitKind::from_token_lit(literal)
        {
            self.input = Some(input.to_string());
        }
        visit::walk_local(self, local);
    }
}

/// 参数宏已经生成 `Option<Injection<T>>` / `Injection<T>`。字段内的 T 仍使用自己
/// 的原始 AST，不能把 `impl<U>` 的 U 复制到 `struct<T>` 中造成跨 owner 泛型引用。
fn rewrite_field(
    resolver: &mut ty::ResolverAstLowering<'_>,
    field: &mut Box<ast::Ty>,
    parameter: &ast::Ty,
) -> Result<(), &'static str> {
    if path_name(parameter) == Some("Option") {
        if path_name(field) != Some("Option") {
            return Err("可选 constructor 参数只能保存到对应 Option<T> 字段");
        }
        let template = first_type(parameter).ok_or("constructor 可选参数类型缺少服务类型")?;
        let inner = first_type_mut(field).ok_or("constructor 可选字段类型缺少服务类型")?;
        return rewrite_field(resolver, inner, template);
    }
    if !matches!(path_name(parameter), Some("Injection" | "LazyInjection")) {
        return Err("constructor 参数未生成预期的 Injection / LazyInjection 类型");
    }
    let mut wrapper = Box::new(parameter.clone());
    let inner = first_type_mut(&mut wrapper).ok_or("constructor 注入包装缺少服务类型")?;
    *inner = field.clone();
    wrapper.id = fresh_node(resolver, wrapper.id);
    if let ast::TyKind::Path(_, path) = &mut wrapper.kind {
        for segment in &mut path.segments {
            segment.id = fresh_node(resolver, segment.id);
        }
    }
    *field = wrapper;
    Ok(())
}

/// 跳过类型外层括号以检查结构，保留原始 AST 不变。
fn unparenthesized_type(mut value: &ast::Ty) -> &ast::Ty {
    while let ast::TyKind::Paren(inner) = &value.kind {
        value = inner;
    }
    value
}

/// 读取去除外层括号后的路径末段；非路径类型没有协议包装名称。
fn path_name(value: &ast::Ty) -> Option<&str> {
    let value = unparenthesized_type(value);
    let ast::TyKind::Path(_, path) = &value.kind else {
        return None;
    };
    Some(path.segments.last()?.ident.name.as_str())
}

/// 读取包装类型的第一个类型实参，不接受 const 或生命周期槽位。
fn first_type(value: &ast::Ty) -> Option<&ast::Ty> {
    let value = unparenthesized_type(value);
    let ast::TyKind::Path(_, path) = &value.kind else {
        return None;
    };
    let ast::GenericArgs::AngleBracketed(args) = path.segments.last()?.args.as_deref()? else {
        return None;
    };
    let ast::AngleBracketedArg::Arg(ast::GenericArg::Type(value)) = args.args.first()? else {
        return None;
    };
    Some(value)
}

/// 定位可改写的第一个类型实参，同时保留外层括号和业务节点身份。
fn first_type_mut(mut value: &mut ast::Ty) -> Option<&mut Box<ast::Ty>> {
    // 只定位泛型槽位；保留业务类型外层括号、路径、NodeId 和 span。
    loop {
        match &mut value.kind {
            ast::TyKind::Paren(inner) => value = inner,
            ast::TyKind::Path(_, path) => {
                let ast::GenericArgs::AngleBracketed(args) =
                    path.segments.last_mut()?.args.as_deref_mut()?
                else {
                    return None;
                };
                let ast::AngleBracketedArg::Arg(ast::GenericArg::Type(value)) =
                    args.args.first_mut()?
                else {
                    return None;
                };
                return Some(value);
            }
            _ => return None,
        }
    }
}

/// 分配新 AST 节点，并复制源节点已有的名称解析结果。
fn fresh_node(resolver: &mut ty::ResolverAstLowering<'_>, source: ast::NodeId) -> ast::NodeId {
    let id = resolver.next_node_id;
    resolver.next_node_id = ast::NodeId::from_u32(id.as_u32() + 1);
    if let Some(resolution) = resolver.partial_res_map.get(&source).copied() {
        resolver.partial_res_map.insert(id, resolution);
    }
    id
}

/// 一次性取得 AST 与 resolver，完成收集和字段重写后交还新的 Steal 结果。
fn lower_constructors<'tcx>(
    tcx: TyCtxt<'tcx>,
    argument: (),
) -> (
    &'tcx Steal<ty::ResolverAstLowering<'tcx>>,
    &'tcx Steal<ast::Crate>,
    &'tcx ty::ResolverGlobalCtxt,
) {
    let (resolver, krate, global) =
        ORIGINAL.get().expect("constructor query hook installed")(tcx, argument);
    let mut resolver = resolver.steal();
    let mut krate = krate.steal();
    let mut declarations = Declarations {
        resolver: &resolver,
        impls: HashMap::new(),
        structs: HashMap::new(),
        struct_parameters: HashMap::new(),
        capture_identifiers: CAPTURE_IDE.load(Ordering::Relaxed),
        identifiers: HashSet::new(),
    };
    declarations.visit_crate(&krate);
    let mut collect = Collect {
        tcx,
        resolver: &resolver,
        declarations: &declarations,
        constructors: HashMap::new(),
    };
    collect.visit_crate(&krate);
    let constructors = collect.constructors;
    let impls = declarations.impls;
    let structs = declarations.structs;
    let identifiers = declarations.identifiers;
    let mut rewrite = Rewrite {
        tcx,
        resolver: &mut resolver,
        impls: &impls,
        constructors: &constructors,
        selected: HashSet::new(),
        inputs: HashMap::new(),
    };
    rewrite.visit_crate(&mut krate);
    for (service, constructor) in &constructors {
        if !rewrite.selected.contains(service) {
            tcx.dcx().span_err(
                constructor.span,
                "#[constructor] 所属结构体必须标注 #[injectable]",
            );
        }
    }
    if CAPTURE_IDE.load(Ordering::Relaxed) {
        capture_model(tcx, &structs, &constructors, &rewrite.inputs, &identifiers);
    }
    (
        tcx.arena.alloc(Steal::new(resolver)),
        tcx.arena.alloc(Steal::new(krate)),
        global,
    )
}

/// 关联 helper 使用 pub(crate) 只为连接同一 crate 中的 struct/impl 私有模块。
/// 这不是新的业务 ABI：按真实生成定义和使用点卫生拒绝普通源码直接调用。
/// 检查位于 typeck 后，能识别 `Service::helper` 的延迟关联项解析以及方法项别名。
pub fn validate(tcx: TyCtxt<'_>) -> bool {
    use rustc_hir::intravisit::{self, Visitor};

    /// 在类型检查后检查 constructor 私有 helper 的真实使用身份。
    struct Audit<'tcx> {
        /// 提供关联项解析结果与诊断的编译上下文。
        tcx: TyCtxt<'tcx>,

        /// 已报告的定义与使用位置，防止重复输出。
        seen: HashSet<(DefId, Span)>,
    }

    impl Audit<'_> {
        /// 拒绝普通源码访问已认证的 constructor 内部 helper，并按位置去重错误。
        fn check(&mut self, definition: DefId, span: Span) {
            let Some(name) = self.tcx.opt_item_name(definition) else {
                return;
            };
            if !matches!(
                name.as_str(),
                ACTIVATE | DEPENDENCIES | CONSTRUCTOR_METADATA
            ) || !internal_access::trusted_span(self.tcx, self.tcx.def_span(definition))
                || internal_access::trusted_span(self.tcx, span)
            {
                return;
            }
            if self.seen.insert((definition, span)) {
                self.tcx.dcx().span_err(span, "constructor 的内部 adapter 和元数据只能由工具链生成代码访问；请调用服务的业务方法");
            }
        }
    }

    impl<'tcx> Visitor<'tcx> for Audit<'tcx> {
        /// 连同嵌套 HIR 项一起审计，防止内部 helper 经嵌套定义访问。
        type NestedFilter = rustc_middle::hir::nested_filter::All;

        /// 向嵌套 HIR 遍历提供同一编译会话。
        fn maybe_tcx(&mut self) -> TyCtxt<'tcx> {
            self.tcx
        }

        /// 检查已解析路径的最终关联项身份和使用位置。
        fn visit_path(&mut self, path: &rustc_hir::Path<'tcx>, _: rustc_hir::HirId) {
            if let Res::Def(_, definition) = path.res {
                self.check(
                    definition,
                    path.segments
                        .last()
                        .map_or(path.span, |segment| segment.ident.span),
                );
            }
            intravisit::walk_path(self, path);
        }

        /// 补查 typeck 才能解析的方法与关联项，权限以最终成员的来源为准。
        fn visit_expr(&mut self, expression: &'tcx rustc_hir::Expr<'tcx>) {
            let owner = expression.hir_id.owner.def_id;
            if self.tcx.has_typeck_results(owner) {
                let typeck = self.tcx.typeck(owner);
                match expression.kind {
                    rustc_hir::ExprKind::Path(path) => {
                        if let Res::Def(_, definition) = typeck.qpath_res(&path, expression.hir_id)
                        {
                            // 拼接路径的整体 span 可能沿用用户 struct 标识符；
                            // 权限只看被访问的最终关联成员，而不是前缀来源。
                            let span = match path {
                                rustc_hir::QPath::Resolved(_, path) => path
                                    .segments
                                    .last()
                                    .map_or(expression.span, |segment| segment.ident.span),
                                rustc_hir::QPath::TypeRelative(_, segment) => segment.ident.span,
                            };
                            self.check(definition, span);
                        }
                    }
                    rustc_hir::ExprKind::MethodCall(segment, ..) => {
                        if let Some(definition) = typeck.type_dependent_def_id(expression.hir_id) {
                            self.check(definition, segment.ident.span);
                        }
                    }
                    _ => {}
                }
            }
            intravisit::walk_expr(self, expression);
        }
    }
    let mut audit = Audit {
        tcx,
        seen: HashSet::new(),
    };
    tcx.hir_walk_toplevel_module(&mut audit);
    audit.seen.is_empty()
}

/// 编辑器消费的是已经由真实类型身份确定的选择，不重新扫描文件或猜测 impl。
/// 原版 rust-analyzer 的标准 proc-macro server 可以按同一个输入校验并重放包装。
fn capture_model(
    tcx: TyCtxt<'_>,
    structs: &HashMap<DefId, rustc_span::Ident>,
    constructors: &HashMap<DefId, Constructor>,
    inputs: &HashMap<DefId, String>,
    identifiers: &HashSet<String>,
) {
    use cargo_nestrs::ide::constructor::{
        ConstructorModel, Declaration, FieldPlan, HelperNames, MethodDeclaration, Selection,
        SourceAnchor, write_constructor_model,
    };
    if std::env::var_os("NESTRS_IDE_CAPTURE").is_none() {
        return;
    }
    let mut model = ConstructorModel::default();
    let source_anchor = |span: Span| {
        let start = tcx.sess.source_map().lookup_char_pos(span.lo());
        let end = tcx.sess.source_map().lookup_char_pos(span.hi());
        let rustc_span::FileName::Real(file) = &start.file.name else {
            return None;
        };
        Some(SourceAnchor {
            file: crate::documentation::remap_source_path(file.local_path()?),
            line: start.line,
            column: start.col.0,
            end_line: end.line,
            end_column: end.col.0,
        })
    };
    if !constructors.is_empty() {
        // RA 的标准宏协议不能传递两个独立展开的 rustc DefId。按当前完整标识符目录
        // 分配一对可命名的编辑器连接，避免给业务成员设置任何永久保留的名称。
        // 单元内共享分配结果，使同位置、同 token 的重复宏展开也得到一致的连接。
        let names = (0u64..)
            .find_map(|index| {
                let activate = format!("__nestrs_ide_constructor_{index}_activate");
                let dependencies = format!("__nestrs_ide_constructor_{index}_dependencies");
                (!identifiers.contains(&activate) && !identifiers.contains(&dependencies))
                    .then_some(HelperNames {
                        activate,
                        dependencies,
                    })
            })
            .expect("finite identifier catalog");
        model.helpers = Some(names);
        for constructor in constructors.values() {
            if let Some(anchor) = source_anchor(constructor.span) {
                model.methods.push(MethodDeclaration {
                    anchor,
                    input: constructor.input.clone(),
                });
            }
        }
        model.methods.sort_by(|left, right| {
            left.anchor
                .cmp(&right.anchor)
                .then_with(|| left.input.cmp(&right.input))
        });
    }
    for (service, input) in inputs {
        let Some(ident) = structs.get(service) else {
            continue;
        };
        let start = tcx.sess.source_map().lookup_char_pos(ident.span.lo());
        let end = tcx.sess.source_map().lookup_char_pos(ident.span.hi());
        let rustc_span::FileName::Real(file) = &start.file.name else {
            continue;
        };
        let Some(file) = file.local_path() else {
            continue;
        };
        let file = crate::documentation::remap_source_path(file);
        let constructor = constructors.get(service);
        let fields = constructor
            .into_iter()
            .flat_map(|constructor| &constructor.fields)
            .map(|(name, (slot, ty))| {
                let optional = path_name(ty) == Some("Option");
                let token = if optional {
                    first_type(ty).unwrap_or(ty)
                } else {
                    ty
                };
                FieldPlan {
                    name: name.clone(),
                    slot: *slot,
                    lazy: path_name(token) == Some("LazyInjection"),
                    optional,
                }
            })
            .collect();
        model.declarations.push(Declaration {
            anchor: SourceAnchor {
                file,
                line: start.line,
                column: start.col.0,
                end_line: end.line,
                end_column: end.col.0,
            },
            input: input.clone(),
            // def_path_str 的 pretty printer 会读取 HIR limits；这里仍在
            // resolver query 内，只能使用不依赖 HIR 的原始 DefPath 数据。
            definition: format!(
                "{}::{}",
                tcx.crate_name(service.krate),
                tcx.def_path(*service)
                    .data
                    .iter()
                    .map(|component| component
                        .data
                        .get_opt_name()
                        .map_or_else(|| "<anonymous>".into(), |name| name.to_string()))
                    .collect::<Vec<_>>()
                    .join("::")
            ),
            selection: Selection {
                constructor: constructor.is_some(),
                fields,
            },
        });
    }
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let rustc_session::config::Input::File(source) = &tcx.sess.io.input else {
        return;
    };
    if let Err(error) = write_constructor_model(&arguments, source, &model) {
        tcx.dcx()
            .err(format!("无法保存 constructor IDE 模型：{error}"));
    }
}
