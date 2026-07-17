# 使用指南

## 选择数据库形态

生产文件数据库使用 `打开` 或 `打开配置`。单元测试优先使用 `打开内存`；需要验证 SQLite 临时文件行为时使用 `打开临时`；多个连接共享内存数据库时使用带 `cache=shared` 的命名 URI。

```yanxu
定 单连接 为 言舟.打开内存（）；
定 共享甲 为 言舟.打开URI（「file:test_shared?mode=memory&cache=shared」）；
定 共享乙 为 言舟.打开URI（「file:test_shared?mode=memory&cache=shared」）；
```

只读服务使用 `打开只读`。URI 的 `mode=ro` 也会自动推导只读旗标，但显式便捷入口更易审计。

## 参数与标识符

值始终放入参数列：

```yanxu
数据库.查询（「SELECT * FROM users WHERE name = ? AND age >= ?」，【姓名，18】）；
```

不要把用户输入拼进 SQL。参数不能替代表名或列名；动态标识符应先由业务白名单确认，再用 `数据库.方言（）.引名（名称）` 引用。

## 预编译与批量

高频同模板操作复用原生预编译对象，并在使用完成后关闭：

```yanxu
定 插入 为 数据库.预编译（「INSERT INTO events(kind, payload) VALUES (?, ?)」）；
逐 事件 于 各事件 则
    插入.执行（【事件【「种类」】，事件【「正文」】】）；
终
插入.关闭（）；
```

`批量执行` 在一个立即事务中提交混合 SQL：

```yanxu
数据库.批量执行（【
    {「SQL」：「INSERT INTO t(v) VALUES (?)」，「参数」：【「甲」】}，
    {「SQL」：「INSERT INTO t(v) VALUES (?)」，「参数」：【「乙」】}
】）；
```

## 事务与保存点

```yanxu
定 事务 为 数据库.立即事务（）；
试 则
    事务.执行（「UPDATE accounts SET balance = balance - ? WHERE id = ?」，【10，甲】）；
    事务.保存点（「credit」）；
    事务.执行（「UPDATE accounts SET balance = balance + ? WHERE id = ?」，【10，乙】）；
    事务.释放点（「credit」）；
    事务.提交（）；
救 所误 则
    若 事务.是否活跃（） 则
        事务.回滚（）；
    终
    抛 所误；
终
```

连接上同一时刻只能有一个顶层事务。保存点释放按后进先出；回滚至保存点后仍须释放或继续使用该保存点。

## 表结构反射

```yanxu
逐 表 于 数据库.表清单（） 则
    言 表；
终

定 结构 为 数据库.表结构（「users」）；
言 结构【「列」】；
言 结构【「索引」】；
言 结构【「外键」】；
```

反射查询本身参数化表名。缺失表返回 `存在 = 假` 的完整空结构，不用解析错误消息。

## 言据存储

规范文本适合跨数据库保真，原生结构适合 JSON1 路径查询：

```yanxu
定 规范值 为 数据库.言据写入值（资料，「规范文本」）；
定 JSON值 为 数据库.言据写入值（资料，「原生结构」）；

数据库.执行（「INSERT INTO profiles(data) VALUES (?)」，【JSON值】）；

定 条件 为 数据库.言据路径（「data」，【「等级」】）.至少（10）；
定 表达式 为 条件.转典（）；
定 SQL 为 （「SELECT data FROM profiles WHERE 」 加 表达式【「SQL」】）；
定 各行 为 数据库.查询（SQL，表达式【「参数」】）.全部（）；
```

JSON1 不可用时 `言据路径` 返回能力错误，不会退化为全表内存过滤。

## 迁移

```yanxu
定 各迁移 为 【
    言舟.迁移（1，「创建用户」，「CREATE TABLE users(id INTEGER PRIMARY KEY)」，「DROP TABLE users」），
    言舟.迁移（2，「增加姓名」，「ALTER TABLE users ADD COLUMN name TEXT」，「ALTER TABLE users DROP COLUMN name」）
】；

定 迁移器 为 言舟.默认迁移器（数据库）；
言 迁移器.检查（各迁移）；
言 迁移器.干运行升级（各迁移）；
迁移器.升级（各迁移）；
```

发布后的迁移内容不可修改。旧 0.1 登记没有校验和时，先审计本地定义，再显式调用 `采用旧校验和`；该操作不会自动猜测或静默采用。

## CLI 兼容后端

```yanxu
定 数据库 为 言舟.打开为（「应用.db」，「/usr/local/bin/sqlite3」，30000）；
```

CLI 适合没有对应原生制品的开发环境和兼容排障。每次请求启动一个进程，性能和事务交互能力都弱于原生后端。顶层应用必须同时授权文件、进程和依赖清单要求的原生扩展权限。
