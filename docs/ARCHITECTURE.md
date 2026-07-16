# 架构

## 分层

```text
应用 / ORM
    -> 言舟 SQLite连接
        -> 言库 函数连接、结果、方言与能力
            -> 原生桥 -> YanXu ABI v2 -> rusqlite -> bundled SQLite
            -> CLI桥  -> 标准:进程 -> sqlite3
```

`言库` 定义数据库通用协议；`言舟` 只实现 SQLite 方言特有的打开、事务、反射、迁移和 JSON1 能力。ORM 模型、关联和连接池不进入驱动。

## ABI v2

原生模块名为 `言舟`，公开打开、查询、执行、信息、关闭、事务、保存点、预编译与反射操作。ABI 只交换固定 C 布局值，不跨边界暴露 Rust 类型。

连接和预编译语句是宿主原生资源。语句资源记录父连接句柄；宿主在连接关闭时级联关闭子资源。显式关闭与资源析构均幂等，泄漏统计用于集成测试。

## 数据路径

SQL 与参数在言序层保持分离，原生桥把参数列编码为 ABI 值，Rust 层映射到 SQLite `NULL/INTEGER/REAL/TEXT/BLOB` 并调用 rusqlite 绑定。大于言序安全整数的 SQLite 整数在读取时转成文字，避免静默精度损失。

查询先收集列名和声明类型，再逐行转换。SQL、参数、行、列和总结果大小都在原生层限制，超限返回稳定资源错误。

## 事务

原生事务直接在同一连接执行 `BEGIN` 或 `BEGIN IMMEDIATE`，查询与写入立即可见。CLI 没有持久标准输入，因此兼容事务只缓存已绑定 SQL，在提交时交给单一 `sqlite3` 进程。

迁移原生路径始终取得立即写锁，锁内重新读取迁移登记并重新规划，避免两个进程按同一旧状态执行。迁移 SQL 与登记变更位于同一事务。

## 反射

反射使用 `pragma_table_list`、`pragma_table_xinfo(?)`、`pragma_index_list(?)`、`pragma_index_xinfo(?)` 和 `pragma_foreign_key_list(?)`。表名通过参数传递；主目录和临时目录使用固定受信任 SQL 分支。

## 言据与 JSON1

规范文本委托言库/言据进行规范序列化。原生结构先转换为普通言序值，再编码为 JSON 文字存入 SQLite；读取反向恢复。JSON 路径构建器分别参数化路径和值，列名通过 SQLite 方言引用。

## 制品

Rust 原生库静态包含 SQLite 源码。六个平台制品作为普通文件提交，清单固定目标、相对路径、大小和 SHA-256。macOS 使用稳定 `@rpath` 安装名和临时签名；Windows 静态链接 CRT；Linux 只依赖基础 glibc 系统库。
