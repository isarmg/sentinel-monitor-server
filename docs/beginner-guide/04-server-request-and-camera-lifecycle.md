# 04. 服务端请求、认证与摄像头管理

## 4.1 正式启动门

Server 在监听前验证不可变发行树、Web fingerprint、数据库路径和锁、当前 Schema、lease 不变量及全部
持久实例授权码可由当前 external key 认证。无法证明任何一项即拒绝，不在请求期间懒修复。

## 4.2 登录链路

```text
{username,password} exact request -> Foundation username normalization
 -> source/canonical-account/global admission -> bounded Argon2
 -> Session digest + CSRF -> Secure Cookie
```

成功响应只有 `{authenticated,user_id,username,role:"admin",csrf_token}` 五个字段。写 API 再验证 Session、
CSRF 和 Origin/Host。forwarded header 只有在明确可信代理边界内才可使用。管理 username 只标识
Administrator；实例授权码和 Client token 不参与这条浏览器登录链。

## 4.3 创建授权实例

菜单中创建实例后，Server 生成一个加密持久的 64 位授权码。Client 可保存多个码，每个码只能与一台摄像机快照绑定。
Client 统一不同品牌的发现、认证、能力和流信息；Server 只接收当前中立模型。
只有尚未生成 Client token 的实例可以配对；同一授权码不能覆盖已配对 Client。Client token 丢失时由管理员更换授权码后重新配对。

## 4.4 更换授权码与删除

更换授权码会清除 installation/token/在线状态、禁用摄像机并排队清理媒体路径；旧 Client 必须使用新码重新配对。
删除已配对实例先标记 revoked，待媒体操作成功且已观测不存在路径后，再删除设备、期望/实际状态和授权实例。

## 4.5 状态查询

浏览器查询安全投影：授权实例、Client 上报的设备身份/能力/流、在线状态和媒体操作状态。响应不得包含
Client 保管的 RTSP/ONVIF 凭据、完整上游错误或播放 signing secret。授权码仅在管理员实例视图中可查看和更换。

## 4.6 重试语义

媒体路径配置是期望态，由 durable operation 与 reconciler 收敛。PTZ 不由 Server 直连设备；它会写入有期限的
`device_commands`，交给该授权实例所属 Client 拉取并回报结果。移动指令不能在结果不确定时自动盲重放。
命令采用至少一次投递，Client 必须按命令 ID 去重。`expires_at` 是 Server 生成的绝对截止时间：Server 只投递未过期命令，Client 在执行设备副作用前再次检查；截止后的迟到回执不能把 expired 改回成功。轮换或撤销会终结旧 pending 命令。`stop` 是安全例外，可在摄像头禁用或状态过旧时发送，但仍要求当前 Client 凭据有效。

## 4.7 响应语义

- `201`：授权实例已创建或 Client 已配对；`202`：PTZ 指令已排队；`204`：撤销或最终删除已提交。
- `401/403`：身份/CSRF/授权失败。
- `409`：当前资源或账户状态冲突。
- `400`：JSON、字段或业务边界验证失败。
- `429/503`：准入或依赖暂时不可用，可按响应策略重试。

## 4.8 调试

使用时间、operation ID 和 camera ID 关联日志，不打印 URL credential。当前只启用 tower TraceLayer，没有
请求 ID 中间件，不能让排障流程依赖不存在的字段。先证明操作是否已持久化，再检查 claim/lease、
MediaMTX 请求与终态事务，最后才看 Web 刷新。

## 4.9 API 变更

同步修改 Rust DTO/路由、Web client、严格测试、发行 API 身份和文档；直接删除旧字段/路径，不注册 alias。
