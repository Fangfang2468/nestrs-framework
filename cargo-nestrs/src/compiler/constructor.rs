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
    def::{DefKind, Res},
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

type LoweringQuery = for<'tcx> fn(
    TyCtxt<'tcx>,
    (),
) -> (
    &'tcx Steal<ty::ResolverAstLowering<'tcx>>,
    &'tcx Steal<ast::Crate>,
    &'tcx ty::ResolverGlobalCtxt,
);
static ORIGINAL: OnceLock<LoweringQuery> = OnceLock::new();
static CAPTURE_IDE: AtomicBool = AtomicBool::new(false);

/// 仅 discovery 使用原始物理源码坐标；最终 overlay 阶段不得覆盖该模型。
pub fn capture_ide(enabled: bool) {
    CAPTURE_IDE.store(enabled, Ordering::Relaxed);
}

pub fn provide(providers: &mut Providers) {
    let _ = ORIGINAL.set(providers.queries.resolver_for_lowering_raw);
    providers.queries.resolver_for_lowering_raw = lower_constructors;
}

struct Constructor {
    span: Span,
    fields: BTreeMap<String, (usize, Box<ast::Ty>)>,
}

/// impl -> 实际 struct 的映射只依赖名称解析，不能在这里调用 type_of 等 HIR 查询。
/// 否则 query 会在自己的结果尚未构造完毕时重入，形成循环。
struct Declarations<'a, 'tcx> {
    resolver: &'a ty::ResolverAstLowering<'tcx>,
    impls: HashMap<DefId, DefId>,
    structs: HashMap<DefId, rustc_span::Ident>,
}
impl<'ast> Visitor<'ast> for Declarations<'_, '_> {
    fn visit_item(&mut self, item: &'ast ast::Item) {
        if let Some(owner) = self.resolver.owners.get(&item.id) {
            match &item.kind {
                ast::ItemKind::Struct(..) => {
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

struct Collect<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    resolver: &'a ty::ResolverAstLowering<'tcx>,
    declarations: &'a Declarations<'a, 'tcx>,
    constructors: HashMap<DefId, Constructor>,
}
impl<'ast> Visitor<'ast> for Collect<'_, '_> {
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
                let Some(service) = service.filter(|_| implementation.of_trait.is_none()) else {
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
                let mut fields = BTreeMap::new();
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
                            fields,
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

fn unraw(name: &str) -> &str {
    name.strip_prefix("r#").unwrap_or(name)
}

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

struct Rewrite<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    resolver: &'a mut ty::ResolverAstLowering<'tcx>,
    impls: &'a HashMap<DefId, DefId>,
    constructors: &'a HashMap<DefId, Constructor>,
    selected: HashSet<DefId>,
    inputs: HashMap<DefId, String>,
}
impl MutVisitor for Rewrite<'_, '_> {
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
                    ast::ExprKind::Block(branch.clone(), None)
                } else {
                    otherwise.kind.clone()
                };
            }
        }
        mut_visit::walk_expr(self, expression);
    }
}
#[derive(Default)]
struct FieldMode {
    mixed: bool,
    input: Option<String>,
}
impl<'ast> Visitor<'ast> for FieldMode {
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

/// 参数宏已经生成 Option<Injection<T>> / Injection<T>。字段内的 T 仍使用自己
/// 的原始 AST，不能把 impl<U> 的 U 复制到 struct<T> 中造成跨 owner 泛型引用。
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
fn path_name(value: &ast::Ty) -> Option<&str> {
    let ast::TyKind::Path(_, path) = &value.kind else {
        return None;
    };
    Some(path.segments.last()?.ident.name.as_str())
}
fn first_type(value: &ast::Ty) -> Option<&ast::Ty> {
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
fn first_type_mut(value: &mut ast::Ty) -> Option<&mut Box<ast::Ty>> {
    let ast::TyKind::Path(_, path) = &mut value.kind else {
        return None;
    };
    let ast::GenericArgs::AngleBracketed(args) = path.segments.last_mut()?.args.as_deref_mut()?
    else {
        return None;
    };
    let ast::AngleBracketedArg::Arg(ast::GenericArg::Type(value)) = args.args.first_mut()? else {
        return None;
    };
    Some(value)
}
fn fresh_node(resolver: &mut ty::ResolverAstLowering<'_>, source: ast::NodeId) -> ast::NodeId {
    let id = resolver.next_node_id;
    resolver.next_node_id = ast::NodeId::from_u32(id.as_u32() + 1);
    if let Some(resolution) = resolver.partial_res_map.get(&source).copied() {
        resolver.partial_res_map.insert(id, resolution);
    }
    id
}

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
        capture_model(tcx, &structs, &constructors, &rewrite.inputs);
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
    struct Audit<'tcx> {
        tcx: TyCtxt<'tcx>,
        seen: HashSet<(DefId, Span)>,
    }
    impl Audit<'_> {
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
        type NestedFilter = rustc_middle::hir::nested_filter::All;
        fn maybe_tcx(&mut self) -> TyCtxt<'tcx> {
            self.tcx
        }
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
) {
    use cargo_nestrs::ide::constructor::{
        ConstructorModel, Declaration, FieldPlan, Selection, SourceAnchor, write_constructor_model,
    };
    if std::env::var_os("NESTRS_IDE_CAPTURE").is_none() {
        return;
    }
    let mut model = ConstructorModel::default();
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
    if let Err(error) = write_constructor_model(&arguments, &model) {
        tcx.dcx()
            .err(format!("无法保存 constructor IDE 模型：{error}"));
    }
}
