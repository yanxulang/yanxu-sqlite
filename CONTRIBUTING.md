# 贡献指南

## 开发环境

- 言序 1.1.12；
- Rust 1.85+ 与 Cargo；
- SQLite 3.38+，仅 CLI 集成测试需要；
- 六平台发布构建额外使用 Rust 目标、Zig 和 Windows SDK 交叉工具。

## 本地检查

从言序多仓工作区根目录执行：

```sh
cargo test --locked --manifest-path yanxu-libraries-workspace/repos/yanxu-sqlite/Cargo.toml
cargo fmt --manifest-path yanxu-libraries-workspace/repos/yanxu-sqlite/Cargo.toml -- --check
cargo clippy --locked --manifest-path yanxu-libraries-workspace/repos/yanxu-sqlite/Cargo.toml --all-targets -- -D warnings
yanxu-libraries-workspace/core-native-patch/target/release/yanxu 查 yanxu-libraries-workspace/repos/yanxu-sqlite/src/言舟.yx
yanxu-libraries-workspace/core-native-patch/target/release/yanxu 行 yanxu-libraries-workspace/repos/yanxu-sqlite/tests/绑定与语句.yx
yanxu-libraries-workspace/core-native-patch/target/release/yanxu 行 yanxu-libraries-workspace/repos/yanxu-sqlite/tests/事务与迁移.yx
```

三个 `integration` 消费者须分别更新锁、运行源码、验证离线锁并构建 Release YXB。不要并行执行会写共享 Git 包缓存的命令。

## 设计要求

- 原生后端是默认生产实现；CLI 只能保持准确标注的兼容语义；
- 参数必须与 SQL 模板分离，路径和标识符必须使用专用构造边界；
- 新原生资源必须有显式关闭、父资源级联和泄漏测试；
- 新配置必须验证冲突组合并提供稳定错误代码；
- 新制品必须匹配目标格式、导出 ABI v2 入口并更新清单哈希和大小；
- 迁移变化必须覆盖失败回滚、并发写锁复查和校验和漂移；
- 文档不得把只构建的平台写成已经运行通过。

每个可独立验证的行为使用单独提交。拉取请求使用普通合并保留提交历史。
