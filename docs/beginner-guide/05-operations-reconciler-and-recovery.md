# 05. 持久操作、协调器与故障恢复

## 5.1 为什么需要 Operation

对 MediaMTX 的请求可能已生效但响应在网络中丢失。同步 handler 无法把这种情况可靠归类为失败。先持久
化意图再异步执行，使重启和用户查询都围绕同一 operation ID。

## 5.2 状态机

```text
pending -> running -> succeeded
                    -> failed
                    -> unknown
```

`failed` 只用于能证明未达到目标或明确业务拒绝；`unknown` 表示副作用无法证明。unknown 不是可自动重试
的普通失败。

## 5.3 Claim 与 Lease

reconciler 在事务内 claim 可执行 operation，并更新全局/资源租约。租约有规范 owner、更新时间和 expiry；
空闲字段必须成对为 NULL。租约防止两个 worker 同时协调同一状态，但不是跨版本兼容机制。

## 5.4 执行顺序

```text
claim -> decrypt current credential -> validate again
 -> call MediaMTX -> classify result -> fenced terminal transaction
```

外部调用不持有长 SQLite 写事务。完成事务必须检查 lease/operation 仍由当前 worker 拥有。

## 5.5 重启恢复

启动先取得独占实例锁，确认旧进程不再持有该数据目录，然后将上一进程遗留的全部 running 转为 unknown，
包括 operation lease 尚未到期的记录。它们的外部效果无法证明，不能等待租约到期后假装从未执行。
pending 仍可继续；全局 reconciler lease 不在启动恢复中清空，新 worker 必须等它到期后再领取。
这与运行期间只回收过期 operation lease 的路径不同：常规调和不会抢占仍有效的 owner，finalize 仍受双重租约 fencing。
系统不把 unknown 当普通失败盲目重发；操作者核对 actual state 后采取新的明确意图。

## 5.6 审计的当前边界

授权实例 create/pair/rotate/revoke/delete 与相关业务变更在同一事务写入 `audit_logs`。
PTZ 记录的是命令已排队，Client 回报结果保存在 `device_commands`。媒体 operation 还有 xcss
durable-operation audit outbox；它与产品业务审计表是两种语义，不得混同。

## 5.7 Reconcile 的边界

协调器可以把已知期望应用到已知实际接口，但不能自动解释未知旧状态、修复 Schema、猜测 Secret 或
覆盖人工修改。发现合同外状态时应报告并停在安全边界。

## 5.8 故障注入

测试调用前失败、远端明确拒绝、响应前断线、响应解析失败、完成事务失败、lease 过期、健康 lease、
进程 kill 和重启。分别断言 operation、desired/applied path 与审计行，不只断言 HTTP mock 次数。

## 5.9 处置 unknown

保持资源写入静止，查询 MediaMTX actual state 和日志，核对 operation request 摘要。若能证明目标已达成，
通过当前受审计流程收敛；否则提交新的明确操作。不得直接改数据库终态。
