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
