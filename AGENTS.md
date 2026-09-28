# Git Commit 规范

## 1. 概述

本项目采用 **Conventional Commits** 规范管理 Git Commit。

Commit Message 用于描述一次提交的目的和影响范围，使 Git History 保持清晰、可读、可追踪。

本规范同时适用于：

* 人工开发
* AI Coding Agent
* 自动化工具
* CI/CD 相关提交

所有提交都应遵循本文档定义的格式和约定。

---

# 2. Commit Message 格式

Commit Message 的基本格式：

```text
<type>(<scope>): <description>
```

例如：

```text
feat(nestrs-di): 增加依赖图构建功能
fix(nestrs-core): 修复服务注册错误
refactor(nestrs-runtime): 重构服务实例化流程
test(nestrs-di): 增加循环依赖测试
docs(nestrs-web): 补充路由使用文档
```

如果提交无法归属于某个明确的 package，可以省略 `scope`：

```text
chore: 更新许可证
docs: 更新项目 README
```

---

# 3. Commit Message 组成

Commit Message 由三个主要部分组成：

```text
type
scope
description
```

完整结构：

```text
<type>(<scope>): <description>
```

例如：

```text
feat(nestrs-di): 增加 Scoped 生命周期支持
```

其中：

```text
feat
```

表示提交类型。

```text
nestrs-di
```

表示受影响的 Cargo package。

```text
增加 Scoped 生命周期支持
```

表示本次提交的具体变化。

---

# 4. Type

`type` 用于表示本次提交的性质。

本项目使用以下 Type：

| Type       | 含义               |
| ---------- | ---------------- |
| `feat`     | 新增功能             |
| `fix`      | 修复 Bug           |
| `refactor` | 重构，不改变功能行为       |
| `perf`     | 性能优化             |
| `test`     | 测试相关修改           |
| `docs`     | 文档相关修改           |
| `build`    | 构建系统、Cargo、依赖等修改 |
| `ci`       | CI/CD 相关修改       |
| `chore`    | 其他维护性修改          |
| `revert`   | 回滚提交             |

除非确有必要，不应创建新的 Type。

---

# 5. feat

`feat` 用于新增功能。

例如：

```text
feat(nestrs-di): 增加依赖注入功能
feat(nestrs-web): 增加 Guard 支持
feat(nestrs-config): 增加环境变量配置
feat(nestrs-runtime): 增加 Scoped 生命周期支持
```

当提交的主要目的在于为用户增加新的能力时，应使用 `feat`。

---

# 6. fix

`fix` 用于修复已有功能中的 Bug 或错误行为。

例如：

```text
fix(nestrs-di): 修复循环依赖检测错误
fix(nestrs-web): 修复路由参数解析错误
fix(nestrs-core): 修复服务注册失败问题
```

`fix` 应用于真正的错误修复，而不是普通代码修改。

---

# 7. refactor

`refactor` 用于代码重构。

重构的主要特征是：

> 改变代码结构，但不改变对外功能或行为。

例如：

```text
refactor(nestrs-di): 分离服务注册与依赖解析
refactor(nestrs-core): 简化服务注册流程
refactor(nestrs-runtime): 重构服务实例化流程
```

如果修改同时引入了新功能，应优先使用 `feat`。

如果修改主要用于修复 Bug，应使用 `fix`。

---

# 8. perf

`perf` 用于性能优化。

例如：

```text
perf(nestrs-di): 减少依赖图构建过程中的内存分配
perf(nestrs-web): 优化路由匹配性能
perf(nestrs-core): 减少服务解析过程中的类型查找
```

只有当提交的主要目的为性能优化时，才使用 `perf`。

---

# 9. test

`test` 用于增加、修改或重构测试。

例如：

```text
test(nestrs-di): 增加循环依赖测试
test(nestrs-di): 增加服务生命周期测试
test(nestrs-web): 增加路由参数测试
```

如果一次提交同时实现功能和测试，通常以功能作为 Type：

```text
feat(nestrs-di): 增加 Scoped 生命周期支持
```

而不是：

```text
test(nestrs-di): 增加 Scoped 生命周期支持
```

---

# 10. docs

`docs` 用于纯文档修改。

例如：

```text
docs(nestrs-di): 补充依赖注入生命周期说明
docs(nestrs-web): 更新路由使用文档
docs: 更新项目 README
```

如果提交同时修改代码和文档，应根据主要修改内容选择 Type。

---

# 11. build

`build` 用于构建系统、Cargo、依赖以及构建相关配置。

例如：

```text
build(framework): 更新 workspace 依赖
build(framework): 更新 Rust toolchain
build(nestrs-di): 更新 crate 依赖
```

典型场景包括：

* 修改 Cargo 配置
* 更新依赖
* 修改 workspace 配置
* 修改构建脚本
* 修改 Rust toolchain
* 修改构建相关配置

---

# 12. ci

`ci` 用于 CI/CD 配置修改。

例如：

```text
ci(framework): 增加 workspace CI 检查
ci(framework): 增加 Clippy 检查
ci(framework): 优化 GitHub Actions 构建流程
```

---

# 13. chore

`chore` 用于无法合理归入其他类型的维护性修改。

例如：

```text
chore(framework): 清理项目配置
chore(framework): 更新项目元数据
chore: 更新许可证
```

`chore` 不应成为默认 Type。

如果修改明确属于 `feat`、`fix`、`refactor`、`build` 等类型，应使用对应类型。

---

# 14. revert

`revert` 用于回滚之前的提交。

例如：

```text
revert(nestrs-di): 回滚依赖解析重构
```

如果需要，可以在 Commit Body 中说明被回滚的 Commit 以及回滚原因。

---

# 15. Scope

## 15.1 基本规则

本项目是 Rust Monorepo，因此：

> **Scope 默认使用 Rust workspace 中 Cargo package 的 package name。**

Scope 表示：

> 本次提交主要影响哪个 Cargo package。

例如 workspace：

```text
nestrs/
├── Cargo.toml
└── crates/
    ├── nestrs-core/
    ├── nestrs-di/
    ├── nestrs-macros/
    ├── nestrs-runtime/
    ├── nestrs-web/
    ├── nestrs-config/
    └── nestrs-logger/
```

对应的 Scope：

```text
nestrs-core
nestrs-di
nestrs-macros
nestrs-runtime
nestrs-web
nestrs-config
nestrs-logger
```

例如：

```text
feat(nestrs-di): 增加依赖图构建功能
fix(nestrs-core): 修复服务注册错误
feat(nestrs-macros): 增加 service 属性宏
refactor(nestrs-runtime): 重构服务实例化流程
feat(nestrs-web): 增加 Guard 支持
```

---

# 16. Scope 必须使用 Package Name

Scope 应与 Cargo package 的名称保持一致。

例如：

```toml
[package]
name = "nestrs-di"
```

那么 Scope 应使用：

```text
nestrs-di
```

Commit：

```text
feat(nestrs-di): 增加依赖解析功能
```

而不是：

```text
feat(di): 增加依赖解析功能
```

也不是：

```text
feat(dependency): 增加依赖解析功能
```

这样可以让 Git History 与 Cargo workspace 的结构直接对应。

---

# 17. Scope 不使用内部 Module

Cargo package 内部可能存在：

```text
nestrs-di/
└── src/
    ├── graph/
    ├── resolver/
    ├── registry/
    └── lifecycle/
```

即使只修改：

```text
src/graph/
```

Scope 仍然应该是：

```text
nestrs-di
```

例如：

```text
feat(nestrs-di): 增加依赖图构建功能
```

而不是：

```text
feat(graph): 增加依赖图构建功能
```

原因是：

> `graph` 是内部 module，而 `nestrs-di` 是独立的 Cargo package。

Scope 应优先反映 package 边界，而不是源码目录边界。

---

# 18. Framework Scope

如果一次提交针对整个 Rust Monorepo，而不是某一个独立 package，则使用：

```text
framework
```

`framework` 是一个特殊 Scope。

它表示：

> 整个 Nestrs framework workspace。

`framework` 不代表某个具体 Cargo package。

---

# 19. 什么时候使用 framework

以下情况通常应该使用：

```text
framework
```

### 19.1 Workspace Cargo.toml

例如修改根目录：

```text
Cargo.toml
```

Commit：

```text
build(framework): 调整 workspace 配置
```

---

### 19.2 Workspace Dependencies

例如：

```toml
[workspace.dependencies]
tokio = ...
```

Commit：

```text
build(framework): 更新 workspace 依赖
```

---

### 19.3 Rust Toolchain

例如修改：

```text
rust-toolchain.toml
```

Commit：

```text
build(framework): 更新 Rust toolchain
```

---

### 19.4 Workspace CI

例如修改整个 workspace 的 CI：

```text
.github/workflows/ci.yml
```

Commit：

```text
ci(framework): 增加 workspace CI 检查
```

---

### 19.5 全局工程配置

例如：

```text
.gitignore
rustfmt.toml
clippy.toml
```

如果其影响范围是整个 workspace：

```text
chore(framework): 调整全局开发配置
```

---

# 20. 多 Package 修改

如果一个提交同时修改多个 Cargo package，需要根据修改的性质决定 Scope。

如果多个 package 的修改属于**同一个整体架构变更**，使用：

```text
framework
```

例如一次 DI 架构重构同时修改：

```text
nestrs-core
nestrs-di
nestrs-runtime
```

可以：

```text
refactor(framework): 重构服务解析架构
```

---

# 21. 多 Package 修改不代表一定使用 framework

如果多个 package 的修改实际上属于不同逻辑，则应该拆分 Commit。

不推荐：

```text
feat(framework): 增加 DI、修改 Logger、修复 Web 路由
```

应该拆分：

```text
feat(nestrs-di): 增加依赖注入功能
refactor(nestrs-logger): 调整日志初始化流程
fix(nestrs-web): 修复路由注册错误
```

因此：

> `framework` 表示一个整体性的 workspace 级变更，而不是“这次修改碰了多个 package”。

---

# 22. Scope 省略

如果提交没有合理的 Scope，可以省略：

```text
chore: 更新许可证
docs: 更新项目 README
```

不要为了填写 Scope 而强行指定一个不准确的 package。

---

# 23. Description

Description 必须：

* 使用中文
* 简洁
* 准确
* 描述实际变化
* 避免无意义的表达
* 避免过多实现细节

推荐：

```text
feat(nestrs-di): 增加依赖图构建功能
fix(nestrs-di): 修复循环依赖检测错误
refactor(nestrs-core): 简化服务注册流程
perf(nestrs-di): 减少依赖解析过程中的内存分配
```

不推荐：

```text
feat(nestrs-di): 做了一些修改
fix(nestrs-di): 修复了一些问题
refactor(nestrs-core): 改了一下代码
```

---

# 24. Description 必须使用中文

本项目规定：

> **Commit Message 的 Description 必须使用中文。**

例如：

正确：

```text
feat(nestrs-di): 增加依赖图构建功能
```

错误：

```text
feat(nestrs-di): add dependency graph
```

但是以下内容可以保留英文：

* Cargo package name
* Rust 类型名称
* API 名称
* crate 名称
* 技术术语
* 属性宏名称
* 库名称

例如：

```text
feat(nestrs-di): 增加 DependencyGraph 支持
feat(nestrs-macros): 修复 #[service] 宏解析错误
feat(nestrs-runtime): 增加 Tokio runtime 支持
```

---

# 25. Description 应描述“变化”

Commit Message 应优先表达：

> 这次提交改变了什么？

而不是：

> 修改了哪些代码？

例如不推荐：

```text
refactor(nestrs-di): 将 Vec 修改为 BTreeSet
```

如果真正的目的在于解决重复依赖：

```text
refactor(nestrs-di): 消除依赖图中的重复节点
```

`BTreeSet` 是实现细节，而“消除重复节点”才是修改的实际目的。

---

# 26. Description 使用明确动词

推荐使用：

```text
增加
支持
修复
移除
重构
优化
简化
调整
补充
```

例如：

```text
feat(nestrs-di): 支持 Trait 类型注入
fix(nestrs-web): 修复请求参数解析
refactor(nestrs-core): 简化服务注册 API
perf(nestrs-di): 优化依赖图遍历
docs(nestrs-di): 补充服务生命周期说明
```

---

# 27. Description 长度

Subject 应保持简洁。

推荐控制在约 **72 个字符以内**。

推荐：

```text
feat(nestrs-di): 增加 Scoped 生命周期支持
```

不推荐：

```text
feat(nestrs-di): 增加 Scoped 生命周期支持并重构服务解析流程同时修复多个生命周期相关问题
```

如果需要表达更多信息，应使用 Commit Body。

---

# 28. 标点符号

Description 末尾不添加句号。

正确：

```text
feat(nestrs-di): 增加 Scoped 生命周期支持
```

不推荐：

```text
feat(nestrs-di): 增加 Scoped 生命周期支持。
```

---

# 29. Emoji

Commit Message 默认不使用 Emoji。

不推荐：

```text
✨ feat(nestrs-di): 增加依赖注入
🐛 fix(nestrs-web): 修复路由错误
```

除非项目维护者明确要求，否则不要使用 Emoji。

---

# 30. Commit Body

当 Subject 无法完整表达修改内容时，可以增加 Commit Body。

格式：

```text
<type>(<scope>): <description>

<body>
```

例如：

```text
feat(nestrs-di): 增加依赖图拓扑排序

使用 Kahn 算法对服务依赖关系进行拓扑排序，
生成服务实例化所需的解析顺序。

同时增加循环依赖检测。
```

Body 可以用于说明：

* 为什么进行修改
* 采用什么设计
* 重要的行为变化
* 兼容性问题
* 需要特别注意的事项

简单的修改不需要 Body。

---

# 31. Breaking Change

如果一次提交会破坏现有 API 或行为兼容性，应标记为 Breaking Change。

格式：

```text
<type>(<scope>)!: <description>
```

例如：

```text
feat(nestrs-di)!: 重构服务注册 API
```

也可以在 Body 中使用：

```text
BREAKING CHANGE:
```

例如：

```text
feat(nestrs-di)!: 重构服务注册 API

BREAKING CHANGE: ServiceRegistry::register_service 已被移除，
请使用 ServiceRegistry::register。
```

Breaking Change 必须明确说明对现有用户造成的影响。

---

# 32. Commit Granularity

一个 Commit 应尽可能表达一个**独立的逻辑变化**。

推荐：

```text
feat(nestrs-di): 增加依赖图构建
test(nestrs-di): 增加依赖图测试
fix(nestrs-di): 修复循环依赖检测
```

不推荐：

```text
feat(framework): 增加依赖图、修改 Logger、修复 Web 路由
```

不同逻辑应拆分为不同 Commit。

---

# 33. 不要机械地按照文件拆分 Commit

Commit 应按照**逻辑边界**划分，而不是按照文件划分。

例如一个完整功能可能同时修改：

```text
nestrs-di/src/graph.rs
nestrs-di/src/resolver.rs
nestrs-di/tests/di.rs
```

如果它们共同实现一个功能，可以使用一个 Commit：

```text
feat(nestrs-di): 增加依赖图构建功能
```

不要机械拆成：

```text
feat(nestrs-di): 修改 graph.rs
feat(nestrs-di): 修改 resolver.rs
test(nestrs-di): 修改测试
```

---

# 34. 一个 Commit 应保持内部一致

Commit 中的所有修改应该围绕同一个逻辑目的。

例如：

```text
feat(nestrs-di): 增加 Scoped 生命周期支持
```

可以包含：

```text
Scoped 生命周期实现
相关类型修改
相关测试
必要的文档
```

但不应该同时包含：

```text
Logger 重构
Web 路由修改
README 样式调整
无关依赖更新
```

---

# 35. AI 提交 Commit

AI Coding Agent 创建 Commit 时，也必须遵守本规范。

AI 在创建 Commit 前，应检查：

```bash
git status
git diff
git diff --staged
```

并根据实际修改内容确定：

```text
type
scope
description
```

不得仅根据用户最初的任务描述猜测 Commit Message。

---

# 36. AI 不得盲目提交全部修改

AI 不应该在没有检查工作区的情况下直接执行：

```bash
git add .
git commit -m "..."
```

或者：

```bash
git add -A
```

如果工作区中存在与当前任务无关的修改，应避免将这些修改加入当前 Commit。

应根据实际修改内容选择需要提交的文件。

---

# 37. AI Commit Message 规则

AI 创建 Commit 时必须满足：

```text
Conventional Commits
        +
正确 Type
        +
正确 Scope
        +
中文 Description
```

例如：

```text
feat(nestrs-di): 增加依赖图构建功能
```

而不是：

```text
feat: add dependency graph
```

也不是：

```text
feat(di): 增加依赖图
```

因为 `di` 不是 Cargo package name。

---

# 38. AI 不得添加额外署名

除非明确要求，否则 AI 不得在 Commit Message 中添加：

```text
Generated by AI
Created by ChatGPT
Co-authored-by: ChatGPT
Co-authored-by: Claude
Co-authored-by: Codex
```

也不得自行添加 Emoji。

---

# 39. 常见正确示例

### 新增功能

```text
feat(nestrs-di): 增加依赖图构建功能
```

### Bug 修复

```text
fix(nestrs-di): 修复循环依赖检测错误
```

### 重构

```text
refactor(nestrs-di): 分离服务注册与依赖解析
```

### 性能优化

```text
perf(nestrs-di): 减少依赖图构建过程中的内存分配
```

### 测试

```text
test(nestrs-di): 增加服务生命周期测试
```

### 文档

```text
docs(nestrs-di): 补充依赖注入生命周期说明
```

### Package 构建

```text
build(nestrs-di): 更新依赖版本
```

### Workspace 构建

```text
build(framework): 更新 workspace 依赖
```

### CI

```text
ci(framework): 增加 workspace CI 检查
```

### 全局维护

```text
chore(framework): 调整全局开发配置
```

### Breaking Change

```text
feat(nestrs-di)!: 重构服务注册 API
```

---

# 40. 常见错误

## 40.1 使用英文 Description

错误：

```text
feat(nestrs-di): add dependency injection
```

正确：

```text
feat(nestrs-di): 增加依赖注入功能
```

---

## 40.2 使用内部模块作为 Scope

错误：

```text
feat(graph): 增加依赖图构建
```

正确：

```text
feat(nestrs-di): 增加依赖图构建
```

---

## 40.3 使用缩写代替 Package Name

如果 Cargo package 是：

```text
nestrs-di
```

错误：

```text
feat(di): 增加依赖注入
```

正确：

```text
feat(nestrs-di): 增加依赖注入
```

---

## 40.4 多 Package 修改却滥用 framework

错误：

```text
feat(framework): 修改 nestrs-di
```

如果实际上只修改了 `nestrs-di`，应该：

```text
feat(nestrs-di): 增加依赖注入功能
```

`framework` 只用于真正的 workspace / framework 级变化。

---

## 40.5 Description 过于模糊

错误：

```text
fix(nestrs-di): 修复问题
```

正确：

```text
fix(nestrs-di): 修复未注册服务导致的解析错误
```

---

## 40.6 滥用 chore

错误：

```text
chore(nestrs-di): 增加依赖注入功能
```

正确：

```text
feat(nestrs-di): 增加依赖注入功能
```

---

## 40.7 将实现细节作为主要描述

错误：

```text
refactor(nestrs-di): 将 Vec 修改为 BTreeSet
```

如果真正目的是消除重复节点：

```text
refactor(nestrs-di): 消除依赖图中的重复节点
```

---

## 40.8 一个 Commit 包含无关修改

错误：

```text
feat(framework): 增加 DI、重构 Logger、修复 Web 路由
```

正确：

```text
feat(nestrs-di): 增加依赖注入功能
refactor(nestrs-logger): 重构日志初始化流程
fix(nestrs-web): 修复路由注册错误
```

---

# 41. 推荐的 Git History

一个典型的 Nestrs 功能开发过程可以形成：

```text
feat(nestrs-di): 增加服务注册功能
feat(nestrs-di): 增加依赖图构建
feat(nestrs-di): 增加依赖拓扑排序
feat(nestrs-di): 增加循环依赖检测
test(nestrs-di): 增加依赖解析测试
fix(nestrs-di): 修复重复依赖导致的解析错误
refactor(nestrs-di): 分离注册图与解析图
perf(nestrs-di): 减少依赖图构建过程中的内存分配
docs(nestrs-di): 补充依赖注入生命周期说明
```

Workspace 级修改：

```text
build(framework): 更新 workspace 依赖
build(framework): 更新 Rust toolchain
ci(framework): 增加 workspace CI 检查
chore(framework): 调整全局开发配置
```

这样的 Git History 可以直接反映 Nestrs 各个 Cargo package 的演进过程。

---

# 42. Commit 快速参考

标准格式：

```text
<type>(<scope>): <中文描述>
```

Type：

```text
feat       新功能
fix        Bug 修复
refactor   重构
perf       性能优化
test       测试
docs       文档
build      构建与依赖
ci         CI/CD
chore      其他维护
revert     回滚
```

Scope：

```text
Cargo package name
```

例如：

```text
nestrs-core
nestrs-di
nestrs-macros
nestrs-runtime
nestrs-web
nestrs-config
nestrs-logger
```

整个 Monorepo：

```text
framework
```

Description：

```text
必须使用中文
```

完整示例：

```text
feat(nestrs-di): 增加依赖图构建功能
fix(nestrs-core): 修复服务注册错误
refactor(nestrs-runtime): 重构服务实例化流程
perf(nestrs-di): 减少依赖解析过程中的内存分配
test(nestrs-di): 增加循环依赖测试
docs(nestrs-di): 补充依赖注入生命周期说明
build(framework): 更新 workspace 依赖
ci(framework): 增加 workspace CI 检查
```

---

# 43. 核心原则

Git Commit 应做到：

> **准确、简洁、可读、可追踪。**

每个 Commit 应能够回答：

```text
这次提交改变了什么？
```

必要时进一步回答：

```text
为什么进行这个修改？
```

Scope 应回答：

```text
哪个 Cargo package 受到影响？
```

如果是整个 Nestrs workspace：

```text
framework
```

因此，本项目推荐的最终 Commit 风格为：

```text
feat(nestrs-di): 增加 Scoped 生命周期支持
fix(nestrs-di): 修复循环依赖检测错误
refactor(nestrs-core): 简化服务注册流程
perf(nestrs-di): 减少依赖解析过程中的内存分配
test(nestrs-di): 增加服务生命周期测试
docs(nestrs-di): 补充依赖注入生命周期说明
build(framework): 更新 workspace 依赖
ci(framework): 增加 workspace CI 检查
```

**Scope 以 Cargo package 为边界，`framework` 表示整个 Nestrs Monorepo；Description 必须使用中文。**

---

# 架构共识

本部分记录已与维护者确认的 Nestrs 架构。应用只依赖 core 与业务库，服务声明、
编译、文档和编辑器所需工具统一由 `cargo nestrs` 管理。私有过程宏桥接供 rustc、
rustdoc 和原版 rust-analyzer 共用，生成后端只有一份；应用不依赖公开宏 package。
AI 修改时必须遵守；后续改变这些边界仍须与维护者确认。

## 1. 总体定位与 package

* Nestrs 以 DI 为根基。用户面向 `nestrs-core` 与 `cargo-nestrs`；`nestrs-tool-bridge`
  是工具包内部的 `publish = false` 构建工件，不是应用依赖或独立公开 API。
* `nestrs-core` 是 DI 容器、linkme 宿主和所有运行期生态库的地基。
* `cargo-nestrs` 是构建工具：Cargo CLI、编译器适配、声明生成、IDE 项目模型和 HTML 图。
  它不是 runtime crate，应用运行时不链接工具实现。
* `cargo-nestrs/internal/bridge` 是标准 proc-macro 薄桥接，委托
  `cargo-nestrs/src/codegen`。CLI 将其以 `nestrs` extern 提供给编译器与编辑器；
  不复制生成逻辑，不恢复公开 `nestrs-macro` 或独立 `nestrs-codegen` package。
* `nestrs-bootstrap` 仍为未来的顶层引导库，负责 Application、配置和生态组合，
  导出 `NestrsFactory`；本阶段不提前实现。
* `example/` 只作多项目父目录，业务示例分别位于其子目录并各有 Cargo.toml、源码与说明。
  故意非法的 DI 声明放在 `cargo-nestrs/tests/fixtures/`，不使正常示例的默认运行或图导出失败。

## 2. 分层与依赖方向

```text
工具内部：nestrs-tool-bridge → cargo-nestrs::codegen → 类型化服务声明
工具编排：cargo nestrs → rustc/rustdoc 的 extern 注入与 rust-analyzer 项目依赖
语义分析：nestrs-driver → 根据真实类型自动生成 binding
运行期：  应用及未来 bootstrap/logger/config → nestrs-core
```

* core 不依赖 CLI、codegen、宏 crate 或 rustc 内部库。
* 所有运行期生态库单向依赖 core，未来 bootstrap 位于其上；生态库不得反向依赖 bootstrap。
* 编译期 codegen 位于 `cargo-nestrs/src/codegen`，只使用生成所需工具；生成代码
  统一引用 `::nestrs_core::__private`。内部 token API 不是稳定用户 API。
* `Provider::{Class, Factory}` 生产实例；`TraitBinding` 只描述 concrete 到 trait
  的类型投影，不是 Provider，不创建另一份实例。

## 3. linkme 与查询根

* linkme 的唯一对外宿主为 `nestrs-core::__private::linkme`；用户不直接配置 linkme。
* 生成的分布式注册固定引用该路径，不能改指向工具或其他运行时库。
* 查询宏由 core 导出，在具体类型调用处生成静态根；闭合泛型可提供描述回调，
  trait/factory-only 类型不被强加 ProviderDefinition 约束。
* 全链接单元合并静态根，包括已编译但未执行的分支；类型不能捕获外层泛型/const
  参数或 impl 的 Self。动态 key 求值一次，只选择冻结路由，不扩展图。
* 内部跨 crate 桥接 ABI 隐藏导出供生成代码使用，不属于稳定公开 API。

## 4. 工具私有桥接与标准宏展开

* 应用使用 `use nestrs::{injectable, factory, primary};` 和短属性，也支持
  `#[nestrs::injectable]` 等完整路径。字段/参数 helper 支持裸名及 `nestrs::` 路径。
* `nestrs` 是工具注入的 extern 名称，不是在 Cargo.toml 中配置的公开宏依赖。
  同名 Cargo 依赖会明确报冲突，不能静默覆盖工具桥接。
* 桥接只适配标准 proc_macro 输入输出；声明分析、字段/签名改写与注册生成复用
  `cargo-nestrs/src/codegen`。组合 primary 时保留属性末段名称；不能承诺任意重命名
  属性之间都可识别身份。crate/module 路径别名与单个宏重命名有对应回归。
* 标准 Rust 宏展开处理 cfg、外部模块、macro_rules 生成项和属性/derive 顺序；
  Injection<T> 字段和 factory frame 借用签名在类型检查前生成。
* 应用经 cargo nestrs check/build/run/test 获取桥接与自动绑定；普通 Cargo 不注入
  该环境。core 和工具自身可以用普通 Cargo 检查。应用级 Clippy 集成尚未交付。
* driver 不注册 `nestrs` 工具属性，不替换原生展开管线或复制 token server。
  编译器适配仍用于语义分析、图入口和 IDE 构建记录，升级时需维护并回归。

## 5. 自动绑定

* 普通 `impl Trait for Concrete` 按实际注入/查询需求自动参与绑定，业务代码不写 bind。
* 候选限于声明 provider、factory 成功类型和已知闭合泛型，不注册任意 impl，
  不猜测泛型实参或枚举无限类型集合。
* adapter 使用真实 Ty/DefId、归一化与 Unsize 求解检查投影，生成真实 typed coercion
  再由 rustc 检查，不伪造 vtable、不延长借用、不绕过可见性。
* 语义发现与最终生成使用两个完整编译阶段。FileLoader 只覆盖编译输入，原始源码
  不修改；生成产物保存在 target，最终编译再次检查缺失绑定。
* type/key 显式 provider 优先于蓝图；key 精确匹配；primary 只解决同 key 的 trait
  多候选；optional 不能隐藏歧义、环或生命周期错误。
* 核心保证以同 crate 为基础。有限的外部泛型能力不等于跨所有依赖 crate 的完整
  候选汇总；下游独有接口需求、上游私有投影等场景必须如实记录支持边界。
* `nestrs::bind` 只保留为文档隐藏的显式绑定 ABI 回归入口，不是推荐业务 API。
  显式 pair 不再自动重复生成，重复显式 binding 仍是 core 图错误。

## 6. DI 门面与静态图

* 服务查询只通过 core 的 `get_required_service!`、`get_service!` 和 keyed 变体；
  不提供普通公开查询方法或单独 register!。
* build/build_with_options、create_scope、service_provider、warm_up 和消费 owner
  的 dispose_async 保留普通方法。引用绑定实际 root/scope owner 的借用期。
* 任何服务构造前验证全部注册及可物化的闭合类型；结构错误在容器构建入口 panic，
  成功后冻结图。之后不再读取 linkme、展开泛型或变更图。
* 图编译、激活任务展开、失败传播和实例释放使用非递归算法。
* Singleton 可以依赖 Transient，但整个激活闭包不得包含 Scoped；需要 Scoped 的
  Transient 只能从 scope 查询。factory 参数同样参与生命周期验证。
* Rust 类型检查、全图结构检查和外部资源初始化是三个不同层级；不能把
  cargo nestrs check/build 成功描述成已运行容器全图检查。

## 7. Tokio、lease 与关闭

* 每 root 一个中央 Tokio 协调器，所有 scope/查询共享默认 32 个构造名额。
  依赖满足立即推进，没有整层屏障；worker 不递归 resolve。
* Lazy 默认；Eager 预热 Singleton 及必要依赖，scope.warm_up 预热 Scoped。
  Singleton 始终在 root 上下文构造；Transient 按每个消费槽位独立构造。
* Injection 和 ErasedServiceRef 持有强 lease；稳定实例地址、真实 factory frame
  和独立于 Tokio 的迭代 ReleaseDomain 维护内存安全。
* Singleton/Scoped 失败缓存至 owner 关闭，Transient 失败只属于该 occurrence。
  factory Result 要求 E: Debug；构造 panic 进入 ResolveError。
* 取消查询仅取消等待，接受的初始化继续。关闭先排空接受的任务，再按 owner
  逆发布顺序逐个完成 cleanup/释放；每 owner 至多一个 cleanup worker，root 等 scopes。
* dispose_async 等待取消不取消关闭；Drop 只发送幂等关闭请求，不新建 runtime 或
  block_on。Tokio 退出后只保证同步安全释放，不能保证异步 cleanup。
* 逃逸 token 延长必要内存存活但不阻塞逻辑关闭；cleanup panic 聚合成 DisposeError，
  其余 cleanup 继续。不添加自动超时或强制终止策略。

## 8. 依赖图 HTML

* HTML/CSS/JavaScript 和文件输出全部归 cargo-nestrs。core 仅提供内部只读静态图
  JSON 和既有验证语义，不再有 graph_output、graph_output_path 或 BuildError::GraphExport。
* cargo nestrs graph 链接真实选定 binary 的注册集合，通过诊断入口导出图，不执行
  业务 main、constructor、factory、Default、value 表达式或 cleanup。
* 省略 --bin 时导出所选 package 的全部 binary；--workspace 导出 workspace 总览。
  default-run 不隐藏其他入口。每个入口独立编译、校验并隔离缓存；页面保留独立节点和
  依赖边，只标记共同 provider 声明的入口归属，不合并成跨入口容器。
* 项目报告保留成功、错误和 required-features 未启用的跳过状态；编译、校验或不支持
  的入口错误不阻断其他入口，写出报告后以非零退出。全部入口失败也生成诊断报告。
  显式 --bin 维持单图失败不覆盖旧输出；无 binary、入口选择无效、元数据查询或写入
  失败不覆盖旧输出。跳过不算错误，报告只有跳过时退出状态仍为 0，不代表验证成功。
  lib-only package 可列入项目清单，但不能虚构其独立服务图。
* 每个 package 独立解析特性；当前拒绝 --workspace --features，提示使用
  -p PACKAGE --features。默认 workspace members 选中多个 package 时也拒绝显式
  features，即使没有传 --workspace。--workspace 的 all-features/no-default-features 按各 package
  分别应用，不承诺复现一次 Cargo workspace 构建的 feature 合并。
* 当前只支持固定 host 可运行的 binary，以及源码中可定位的 main；宏生成 main、
  lib/test/example 图目标和跨 target 运行尚未支持。无直接 core 依赖、no_main 及
  cfg_attr 引入的 no_main 在执行前拒绝，不能声称全部入口都已验证。
* 页面展示 provider 声明、槽位与投影关系，不是实例状态；重复 Transient 输入仍独立构造。
* 默认写入 Cargo target 的 nestrs-di.html，输出错误由 CLI 报告，不污染容器构建契约。

## 9. 工具链、IDE 与验证

* pin 以 cargo-nestrs/toolchain.json 的 release、完整 commit 与支持的 host 为准。
  当前本机适配为 x86_64-unknown-linux-gnu 与 x86_64-pc-windows-msvc；driver、bridge
  和 sysroot 必须属于实际 host。不匹配时失败，不静默使用默认新编译器或退回源码扫描。
  Windows 使用本机 exe/dll 和 MSVC 工具，不要求 WSL，不宣称 Windows GNU、ARM64
  或跨 target graph/IDE 已支持。
* compiler-driver feature 隔离 rustc_private；普通 core/工具单元测试无需该 feature。
  zyn 是共享生成后端的基础依赖；内部 bridge 不建立用户面向的宏 feature 契约。
* tools/build-toolchain.py 构建 CLI、driver 和匹配的 bridge。bootstrap 授权限于
  nestrs_driver 构建，不改变全局工具链，也不向应用传播该变量。
* CLI 按完整编译器身份及 driver、bridge 的联合内容指纹隔离 target；Cargo 保留
  构建单元复用，rustc incremental 当前关闭，不宣称已有完整增量事务协议。
* 编译器和 rustdoc 同时获得 bridge 所在目录的 dependency 搜索路径，使没有直接
  core 依赖的下游也能解码上游 metadata。不能只给 producer 注入一个 extern 别名。
* rustdoc 经 driver 注入同一 bridge 后转发给固定 sysroot 的真实 rustdoc；不使用
  拒绝 shim，不静默跳过 doctest。独立 doctest 内新增服务的自动 trait 绑定不经过
  两阶段 driver，不能把标准声明展开等同于新增接口自动注册。
* `cargo nestrs init` 面向手动组装后接入 Nestrs 的现有 Rust 项目，初始化或刷新开发
  环境；默认生成通用 rust-analyzer 项目与设置，只有 `--vscode` 才写 VS Code 配置。
  init 不创建项目、不添加 Cargo 依赖、不安装编辑器或工具链组件；其他 rust-analyzer
  LSP 客户端仍需自行加载生成配置，不能承诺所有支持 Rust 的编辑器都自动接入。
* 未来 `cargo nestrs create` 在 bootstrap 完成后负责创建项目，内部复用初始化能力，
  直接交付已初始化项目，用户无需再执行 init。create 当前属于规划，本阶段不实现。
* cargo nestrs init 依据 Cargo artifacts 与真实 rustc 单元生成 rust-project.json，
  保留依赖重命名/版本、cfg、edition、test、build.rs 环境、OUT_DIR 及过程宏工件。
  为 core 用户增加编辑器专用的 `nestrs` 宏依赖，使用原版宏服务器。
  Windows 编辑器路径统一为普通盘符/UNC，与正常文件 URI 对应；不能只改测试 URI
  规避 verbatim 路径造成的 VFS 身份差异。设备或仅 verbatim 可表示的路径明确拒绝。
  内部 artifact/缓存身份继续归一化，首次准备与保存检查必须复用同一模型与缓存。
* cfg 由同一 rustc 按实际参数执行 --print cfg 获取；cfg.setTest=false 和
  cargo.cfgs=[] 阻止编辑器合成 test、debug_assertions 或 miri 条件。原版
  rust-analyzer 仍合并 host 默认 cfg，因此 IDE 拒绝 panic=abort、禁用默认 CPU
  特性等移除默认条件的配置并保留旧模型。debug/release 和增加 CPU 特性可表示；
  此限制不影响应用 check/build/run。
* init --vscode 合并 linkedProjects、check override 和宏服务器配置，保留无关设置、
  JSONC 注释与已有诊断偏好；不新增诊断屏蔽。保存时 init check 成功后刷新模型，
  失败保留上一份；未保存源码由 rust-analyzer 自身分析。详情见 docs/NESTRS_IDE.md。
  check.extraEnv 固定实际选定的 rustc、driver、bridge 路径并保留其他用户环境变量。
* tools/verify-ide.py 已验证真实原版 LSP 的冷启动、字段/工厂类型、补全、定义跳转、
  未保存编辑、真实错误与恢复，以及 feature、宏生成项与 build.rs 产物；仍不能宣称
  所有编辑器 UI、重命名操作、属性组合或其他 host 都已验收。
* native_host、bridge_metadata、rustdoc 集成测试在 Linux/Windows 均启用；前者运行
  真实 CLI 检查/构建/运行、图副作用隔离与重复导出、IDE 项目生成和配置的保存检查，
  目录包含空格与中文。项目模型回归不等于完整 LSP 交互验收；不得把 Linux 结果当作
  Windows 实机结果，实际验收范围须分别记录。
* 2026-09-28 已实际通过 Windows MSVC workspace check/test、DI 与 56 个 UI 契约、
  Eager/scope 预热示例及 cleanup、原生集成测试、rustdoc、图 12 条命令和跨 crate
  6 次 debug/release 运行。原版 rust-analyzer 0.3.3049 的 default/alternate/release
  完整 LSP 验收通过，使用普通 Windows 文件 URI、未向 RA 父进程额外加入 sysroot/bin。
  此证据仍不覆盖所有编辑器 UI、重命名操作或其他 host。native-host CI 定义双 host
  回归，工作流文件已提供不等于对应提交的 GitHub Actions 已成功执行。
* DI fixture 保留原 52 个 UI 语义基线，加 3 个宏/helper 误用和 1 个导入成功用例。
  UI 经 CLI 私有 bridge 编译；保留旧错误语义，不批量覆盖 stderr 掩盖退化。
* tools/verify-graph.py 验证副作用隔离、不同 package/binary 缓存、项目部分失败和全部
  失败报告、feature 跳过、入口拒绝与文件导出。单图验证失败保留旧输出；项目报告保留
  已验证图与独立诊断。不能运行原业务入口代替诊断入口。

## 10. 未来生态与命名

* 顶层引导库继续命名 nestrs-bootstrap，不使用暗示底层公共库的 nestrs-common。
* 未来 bootstrap 对接 logger/config/DI 等生态并聚合相应 features；运行期生态库
  建立在 core 之上，bootstrap 位于最上层。
* 本次工具链整合不增加 runtime crate，不提前实现 bootstrap、动态注册、运行期扩图
  或集合解析。

## 11. 当前说明与历史记录

当前使用和限制以 [Cargo 工具链说明](docs/NESTRS_CARGO_TOOLCHAIN.md) 为准；
[完整演进方案](docs/NESTRS_COMPILER_TOOLCHAIN_PLAN.md) 记录后续验收。
阶段 B、独立 codegen 提取、公开薄宏和原生工具属性路线均属于历史记录。
当前标准宏桥接由 CLI 私有管理，生成后端在 cargo-nestrs，HTML 在 CLI；不能据历史
文件恢复公开宏依赖、独立后端包、原生展开替换或 core HTML 配置。IDE 以
[当前接入说明](docs/NESTRS_IDE.md) 和真实 LSP 验证为准。
