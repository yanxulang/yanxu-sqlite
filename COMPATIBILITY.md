# 兼容性

## 言序与依赖

| 项目 | 支持范围 | 说明 |
| --- | --- | --- |
| 言序 | `>=1.1.12` | 需要 ABI v2、原生资源父子生命周期和路径级文件权限 |
| 言库 | `^1.0` | 锁定 `v1.0.0` 稳定协议 |
| 原生 SQLite | 3.53.2 | 由 `libsqlite3-sys 0.38.1` bundled 源码固定构建 |
| CLI SQLite | 3.38+ | 需要 `-json` 和 pragma table-valued functions |

言舟依赖 `标准:原生` 模块，只支持字节码 VM、YXB 和包运行路径。树解释器不提供该标准模块，即使代码只选择 CLI 后端，也不能载入言舟；本地验证应使用 `yanxu 行` 或先 `yanxu 编` 再运行。

## 原生制品

| 系统 | 架构 | 目标 | 本轮验证 |
| --- | --- | --- | --- |
| macOS | ARM64 | `aarch64-apple-darwin` | 构建、签名、ABI 导出、真实消费者运行 |
| macOS | x86-64 | `x86_64-apple-darwin` | 交叉构建、临时签名、Mach-O 与 ABI 导出 |
| Linux | ARM64 | `aarch64-unknown-linux-gnu` | 交叉构建、ELF 与 ABI 导出 |
| Linux | x86-64 | `x86_64-unknown-linux-gnu` | 交叉构建、ELF 与 ABI 导出 |
| Windows | ARM64 | `aarch64-pc-windows-msvc` | 交叉构建、PE 与 ABI 导出、静态 CRT |
| Windows | x86-64 | `x86_64-pc-windows-msvc` | 交叉构建、PE 与 ABI 导出、静态 CRT |

只有 macOS ARM64 已在本地真实加载执行；其他平台的运行验证由对应 CI runner 完成后才计为已运行。所有清单制品均有固定 SHA-256 和大小门禁。

## 后端能力

| 能力 | 原生后端 | CLI 兼容后端 |
| --- | --- | --- |
| 参数化查询 | SQLite 原生绑定 | SQL 感知安全字面绑定 |
| 真正预编译资源 | 是 | 否，仅兼容外观 |
| 交互事务查询 | 是 | 否，提交后查询 |
| 保存点 | 是 | 是，提交时脚本 |
| 表结构反射 | 是 | SQLite 3.38+ 可用 |
| JSON1 探测 | 是 | 不声明 |
| 言据原生结构 | JSON1 可用时 | 不声明 |
| WAL/外键/busy timeout | 打开配置 | 由显式 SQL/CLI 进程边界管理 |
| URI/临时/只读便捷入口 | 是 | 使用 `打开为` 自行配置路径 |

## 1.x 承诺

1.x 保持公开类型、法名、配置键、结果字段、错误代码前缀、迁移表结构和默认安全边界。新增可选配置、能力或表达式属于兼容变化；删除公开入口、改变默认打开模式、放宽 URI 权限检查或改变迁移校验语义需要新的主版本。
