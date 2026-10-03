//! 基于 rustc 名称解析结果追踪构造参数的整值来源。
//!
//! 本阶段的 AST 已完成 cfg 筛选和标准宏展开。绑定使用 Res::Local 指向的 NodeId，
//! 而非源码拼写或 span 的近似比较：普通遮蔽、宏定义端变量、调用端捕获变量自然具有
//! 各自的身份。只有直接保存参数/简单别名的字段才包装；依赖声明本身始终完整保留。

use super::service_resolution;
use rustc_ast::{
    ast,
    visit::{self, Visitor},
};
use rustc_hir::{
    def::{CtorOf, DefKind, Res},
    def_id::DefId,
};
use rustc_middle::ty::{self, TyCtxt};
use rustc_span::Span;
use std::collections::{BTreeMap, HashMap};

type Fields = BTreeMap<String, usize>;
type Bindings = HashMap<ast::NodeId, Origin>;
type Analysis<T> = Result<T, (Span, &'static str)>;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Origin {
    Parameter(usize),
    Service(Fields),
    Failure,
    Other,
    Diverges,
}
impl Origin {
    fn tracked(&self) -> bool {
        matches!(self, Self::Parameter(_) | Self::Service(_))
    }
}

pub(super) fn fields<'tcx>(
    tcx: TyCtxt<'tcx>,
    resolver: &ty::ResolverAstLowering<'tcx>,
    impls: &HashMap<DefId, DefId>,
    service: DefId,
    function: &ast::Fn,
    fallible: bool,
) -> Analysis<Fields> {
    let mut bindings = Bindings::new();
    for (slot, parameter) in function.sig.decl.inputs.iter().enumerate() {
        if let Some(binding) = local(resolver, parameter.pat.id) {
            bindings.insert(binding, Origin::Parameter(slot));
        }
    }
    let mut analyzer = Analyzer {
        tcx,
        resolver,
        impls,
        service,
        returns: Vec::new(),
    };
    let body = function
        .body
        .as_ref()
        .ok_or((function.ident.span, "constructor 必须包含函数体"))?;
    let tail = analyzer.block(body, &mut bindings)?;
    analyzer.returns.push(tail);
    let mut fields = None;
    for origin in analyzer.returns {
        match origin {
            Origin::Service(candidate) => {
                if fields.as_ref().is_some_and(|fields| *fields != candidate) {
                    return Err((
                        body.span,
                        "constructor 的成功返回分支具有不同的参数到字段映射；请让同一字段始终接收同一依赖参数",
                    ));
                }
                fields = Some(candidate);
            }
            Origin::Failure if fallible => {}
            Origin::Diverges => {}
            _ => {
                return Err((
                    body.span,
                    "无法证明 constructor 返回值的字段来源；请直接返回 Self { ... } 或 Ok(Self { ... })，可使用一致的 if / match 分支",
                ));
            }
        }
    }
    // 始终失败的 Result 构造函数没有成功存储字段，但仍有全部构造参数输入。
    Ok(fields.unwrap_or_default())
}

fn local(resolver: &ty::ResolverAstLowering<'_>, node: ast::NodeId) -> Option<ast::NodeId> {
    match resolver.partial_res_map.get(&node)?.base_res() {
        Res::Local(binding) => Some(binding),
        _ => None,
    }
}

struct Analyzer<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    resolver: &'a ty::ResolverAstLowering<'tcx>,
    impls: &'a HashMap<DefId, DefId>,
    service: DefId,
    returns: Vec<Origin>,
}
impl Analyzer<'_, '_> {
    fn block(&mut self, block: &ast::Block, bindings: &mut Bindings) -> Analysis<Origin> {
        let mut value = Origin::Other;
        for statement in &block.stmts {
            if value == Origin::Diverges {
                break;
            }
            value = match &statement.kind {
                ast::StmtKind::Let(declaration) => {
                    let origin = match declaration.kind.init() {
                        Some(initializer) => self.expression(initializer, bindings)?,
                        None => Origin::Other,
                    };
                    if origin == Origin::Diverges {
                        return Ok(origin);
                    }
                    if let ast::LocalKind::InitElse(_, otherwise) = &declaration.kind {
                        self.block(otherwise, &mut bindings.clone())?;
                    }
                    self.bind(&declaration.pat, origin, bindings)?;
                    Origin::Other
                }
                ast::StmtKind::Expr(expression) => self.expression(expression, bindings)?,
                ast::StmtKind::Semi(expression) => {
                    let origin = self.expression(expression, bindings)?;
                    if origin == Origin::Diverges {
                        origin
                    } else {
                        Origin::Other
                    }
                }
                // 内部 item 不能捕获外层参数，其 return 不属于当前函数。
                ast::StmtKind::Item(_) | ast::StmtKind::Empty => Origin::Other,
                ast::StmtKind::MacCall(_) => {
                    return Err((statement.span, "constructor 来源分析要求宏完成标准展开"));
                }
            };
        }
        Ok(value)
    }

    fn bind(&self, pattern: &ast::Pat, origin: Origin, bindings: &mut Bindings) -> Analysis<()> {
        match &pattern.kind {
            ast::PatKind::Ident(_, _, None) => {
                if let Some(binding) = local(self.resolver, pattern.id) {
                    bindings.insert(binding, origin);
                }
            }
            ast::PatKind::Wild => {}
            _ if origin.tracked() => {
                return Err((
                    pattern.span,
                    "constructor 依赖参数的别名只支持简单 let 绑定，不能通过解构推断字段来源",
                ));
            }
            _ => {}
        }
        // 复杂普通模式绑定的 NodeId 不会覆盖任何外层绑定，无需按名字删除旧来源。
        Ok(())
    }

    fn expression(&mut self, expression: &ast::Expr, bindings: &mut Bindings) -> Analysis<Origin> {
        use ast::ExprKind as E;
        match &expression.kind {
            E::Path(..) => {
                if self.own_service(expression.id) {
                    return Ok(Origin::Service(Fields::new()));
                }
                Ok(local(self.resolver, expression.id)
                    .and_then(|id| bindings.get(&id))
                    .cloned()
                    .unwrap_or(Origin::Other))
            }
            E::Paren(inner) => self.expression(inner, bindings),
            E::Block(block, _) if block.rules == ast::BlockCheckMode::Default => {
                self.block(block, &mut bindings.clone())
            }
            E::Struct(structure) if self.own_service(expression.id) => {
                if !matches!(structure.rest, ast::StructRest::None) {
                    return Err((
                        expression.span,
                        "constructor 字面量不支持 .. 更新语法；请明确写出各字段来源",
                    ));
                }
                let mut fields = Fields::new();
                // 被 cfg 删除的字段已不在 AST 中，不能把源 token 中的禁用字段重新加回来。
                for field in &structure.fields {
                    if let Origin::Parameter(slot) = self.expression(&field.expr, bindings)? {
                        fields.insert(field.ident.name.to_string(), slot);
                    }
                }
                Ok(Origin::Service(fields))
            }
            E::Ret(value) => {
                let origin = match value {
                    Some(value) => self.expression(value, bindings)?,
                    None => Origin::Other,
                };
                self.returns.push(origin);
                Ok(Origin::Diverges)
            }
            E::If(condition, yes, no) => {
                self.expression(condition, bindings)?;
                let yes = self.block(yes, &mut bindings.clone())?;
                let no = match no {
                    Some(no) => self.expression(no, &mut bindings.clone())?,
                    None => Origin::Other,
                };
                merge(yes, no, expression.span)
            }
            E::Match(value, arms, _) => {
                self.expression(value, bindings)?;
                let mut merged = Origin::Diverges;
                for arm in arms {
                    let mut arm_bindings = bindings.clone();
                    if let Some(guard) = &arm.guard {
                        self.expression(&guard.cond, &mut arm_bindings)?;
                    }
                    if let Some(body) = &arm.body {
                        let origin = self.expression(body, &mut arm_bindings)?;
                        merged = merge(merged, origin, expression.span)?;
                    }
                }
                Ok(merged)
            }
            E::Call(function, arguments) => {
                if arguments.len() == 1 {
                    if self.result_variant(function, "Ok") {
                        return self.expression(&arguments[0], bindings);
                    }
                    if self.result_variant(function, "Err") {
                        self.expression(&arguments[0], bindings)?;
                        return Ok(Origin::Failure);
                    }
                }
                self.expression(function, bindings)?;
                for argument in arguments {
                    if self.expression(argument, bindings)?.tracked() {
                        return Err((
                            argument.span,
                            "无法证明 constructor 调用后的依赖令牌来源；请直接保存参数或简单 let 别名，业务计算可借用参数",
                        ));
                    }
                }
                Ok(Origin::Other)
            }
            E::Assign(left, right, _) | E::AssignOp(_, left, right) => {
                let left = self.expression(left, bindings)?;
                let right = self.expression(right, bindings)?;
                if left.tracked() || right.tracked() {
                    return Err((
                        expression.span,
                        "constructor 依赖参数或构造结果的别名不支持重新赋值；请使用简单 let 绑定表达唯一来源",
                    ));
                }
                Ok(Origin::Other)
            }
            E::MethodCall(call) => {
                self.expression(&call.receiver, bindings)?;
                for argument in &call.args {
                    if self.expression(argument, bindings)?.tracked() {
                        return Err((
                            argument.span,
                            "无法证明 constructor 方法调用后的依赖令牌来源；请直接保存参数或简单 let 别名，业务计算可借用参数",
                        ));
                    }
                }
                Ok(Origin::Other)
            }
            E::Field(value, _) | E::AddrOf(_, _, value) | E::Unary(_, value) | E::Try(value) => {
                self.expression(value, bindings)?;
                Ok(Origin::Other)
            }
            E::Binary(_, left, right) => {
                self.expression(left, bindings)?;
                self.expression(right, bindings)?;
                Ok(Origin::Other)
            }
            E::Let(pattern, value, ..) => {
                let origin = self.expression(value, bindings)?;
                self.bind(pattern, origin, bindings)?;
                Ok(Origin::Other)
            }
            E::Loop(..) | E::While(..) | E::ForLoop { .. } => Err((
                expression.span,
                "constructor 的字段来源分析暂不支持循环；请将循环业务计算提取为普通辅助函数",
            )),
            _ => {
                self.reject_hidden(expression, bindings)?;
                Ok(Origin::Other)
            }
        }
    }

    fn own_service(&self, node: ast::NodeId) -> bool {
        if service_resolution(self.resolver, self.impls, node) == Some(self.service) {
            return true;
        }
        matches!(self.resolver.partial_res_map.get(&node).map(|r| r.base_res()), Some(Res::Def(DefKind::Ctor(CtorOf::Struct, _), definition)) if self.tcx.parent(definition) == self.service)
    }

    /// Result 变体必须是名称解析得到的 core 原生定义，不能把同名业务函数当作 Ok/Err。
    /// 这里只读外部 DefPath；调用 HIR/typeck/lang_items 查询会重入正在构建的 lowering。
    fn result_variant(&self, expression: &ast::Expr, name: &str) -> bool {
        let Some(resolution) = self.resolver.partial_res_map.get(&expression.id) else {
            return false;
        };
        let Res::Def(DefKind::Ctor(CtorOf::Variant, _), definition) = resolution.base_res() else {
            return false;
        };
        if definition.is_local() || self.tcx.crate_name(definition.krate).as_str() != "core" {
            return false;
        }
        let definition = self.tcx.parent(definition);
        let names = self
            .tcx
            .def_path(definition)
            .data
            .into_iter()
            .filter_map(|part| part.data.get_opt_name())
            .collect::<Vec<_>>();
        names
            .iter()
            .map(|name| name.as_str())
            .eq(["result", "Result", name])
    }

    fn reject_hidden(&self, expression: &ast::Expr, bindings: &Bindings) -> Analysis<()> {
        struct Hidden<'a, 'tcx> {
            resolver: &'a ty::ResolverAstLowering<'tcx>,
            bindings: &'a Bindings,
            found: Option<Span>,
        }
        impl<'ast> Visitor<'ast> for Hidden<'_, '_> {
            fn visit_expr(&mut self, expression: &'ast ast::Expr) {
                if local(self.resolver, expression.id)
                    .and_then(|id| self.bindings.get(&id))
                    .is_some_and(Origin::tracked)
                {
                    self.found = Some(expression.span);
                }
                visit::walk_expr(self, expression);
            }
            fn visit_item(&mut self, _: &'ast ast::Item) {}
        }
        let mut hidden = Hidden {
            resolver: self.resolver,
            bindings,
            found: None,
        };
        hidden.visit_expr(expression);
        if let Some(span) = hidden.found {
            Err((
                span,
                "constructor 依赖参数不能隐藏在无法分析的表达式中；请用简单 let 和直接字段赋值明确来源",
            ))
        } else {
            Ok(())
        }
    }
}

fn merge(left: Origin, right: Origin, span: Span) -> Analysis<Origin> {
    if left == right || right == Origin::Diverges || right == Origin::Failure {
        return Ok(left);
    }
    if left == Origin::Diverges || left == Origin::Failure {
        return Ok(right);
    }
    if left.tracked() || right.tracked() {
        return Err((
            span,
            "constructor 分支的依赖参数来源不一致；同一字段必须在所有成功路径接收同一参数",
        ));
    }
    Ok(Origin::Other)
}
