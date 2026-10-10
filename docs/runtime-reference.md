# xcos 运行状态与协议参考

本文供接口、协调器与运维诊断使用。首次部署见[安装指南](getting-started.md)，摄像头操作见[使用指南](usage.md)。

## 媒体操作人工处理与审计

当前 `POST /api/v1/media/operations/{id}/resolve` 要求管理员 Session 和 CSRF，严格请求体为
`{"resolution":"confirmed_succeeded"}`、`confirmed_failed` 或 `unable_to_confirm` 三种决定之一。
确认成功/失败将 Unknown、Failed 或 DeadLetter 标为 Resolved；无法确认仅将 Unknown 标为 DeadLetter。
这些决定不重放原请求。处理前必须核对 MediaMTX 的实际状态；操作者审计与平台状态同事务提交。

调和结果、摄像头状态、当前 owner 的完成转换与 outbox 同事务。启动取得独占实例锁后，上一进程遗留的
所有 running 操作都进入 Unknown，包括尚未到期的操作租约；全局 lease 保留至到期。运行期仅回收过期操作，
仍有效的 owner 不被抢占。租约过期或本地结果提交失败进入 Unknown，
不作为可重试失败。`operation-audit` 监督任务按事件 ID 幂等物化审计；插入和 outbox 确认同事务。
投递失败会将该 Degrading 任务标为失败并保留积压，诊断可见；排除存储故障后重启恢复投递。

构建与平台要求见[开发指南](development.md)。

## 摄像机状态与观测时效

`GET /api/v1/cameras` 的 `status` 是最近保存的媒体状态，`device_status` 是 Client 最近上报的设备状态，
两者都不能脱离 `observation_status` 被解释为当前状态：

- `fresh`：最近一次完整的 MediaMTX 路径清单已成功保存，且观测仍在有效期内。
- `stale`：曾有成功观测，但清单请求失败、服务重启、媒体配置重新调和，或有效期已结束；保留最后已知状态。
- `unknown`：尚无成功的媒体观测。依赖不可用不能证明设备离线。

`last_observed_at` 是最后一次成功保存的清单观测的请求开始时间（Server UTC），在线与离线观测都会更新它；
失败不更新。`observation_expires_at` 是其有效期上限，按请求开始时间加上
`3 × status_interval_secs + request_timeout_secs` 计算。失败或启动时将有效期清空，但保留成功时间与状态；
API 按当前时间再次判定过期，时钟回拨到观测之前也不能把它当作新观测。
`last_seen_at` 仍可能由 Client 快照刷新，不表示媒体清单读取成功，不能用作媒体观测时间。

Web 显示“观测已过期”或“尚无有效观测”，详情同时显示最后有效媒体观测时间、保存的媒体状态和 Client
上报的设备状态。已停用设备保持“已停用”。浏览器快照也在服务端给出的有效期结束后失效；即使没有新响应，
旧在线徽标也不会一直保留。`/v3/info` 成功只说明 MediaMTX 信息接口可用，不能证明路径清单或摄像机在线。

当前数据库格式为 `xcos-db-v2`、Schema revision `2`，浏览器契约为 `xcos-wire-v2`；Client 设备协议和媒体 JWT
未变化。运行前校验当前结构，失败时保留原数据库。全新部署通过显式 `xcos init` 创建当前状态。

## 4. 当前数据库与凭据合同

`product_metadata` 必须恰好一行：

```text
application=xcos
application_version=xcos-db-v2
schema_revision=2
schema_sha256=4d20083821ff39d78792d0795b26206e851c2e6d0523109ee49cfc06666a1d4a
```

xcss `_common_administrators` 表保存不透明 TEXT `administrator_id`、canonical `username`、密码摘要、启停状态、Session version 和微秒整数时间
字段，不保存 email 或 `role`。username 的 Schema CHECK 精确要求 3–64 bytes、ASCII 小写、首尾
`[a-z0-9]`、其余字符仅 `[a-z0-9._-]`；唯一索引直接作用于 canonical 值。登录 candidate 可包含首尾
ASCII whitespace/大写，但 xcss 规范化后才查询；`@`、Unicode、内部空白、控制字符和首尾分隔符
都会拒绝。所有成功认证的控制面账户都是 Administrator；禁止通过手改表或增加配置字段制造身份等级。

`media_reconciler_leases` 必须是当前固定结构且恰有 `singleton=1`。空闲 owner/expiry 同为 NULL；持有态
owner 是规范 UUIDv4，时间为 UTC RFC 3339 且 expiry 晚于 updated。产品不会修补非法状态。

所有敏感字段必须是当前规范 envelope，并能用当前 external key 解密。不要直接编辑数据库或复制密文
字段。

## 6. 锁顺序

应用全生命周期持有数据库 instance 排他、maintenance 共享和 runtime app lock。正常运行持有数据库父目录
`.state-maintenance.lock` 共享锁与 `.state-instance.lock` 独占锁；runtime 目录使用同一公共锁协议并维护 PID。
MediaMTX 由 `flock --no-fork` 持有 companion lock。维护工具必须按database maintenance -> runtime -> MediaMTX
取得排他锁。不要用不同 runtime 指向同一数据库；database identity lock 仍会拒绝第二实例。

## 日志与历史容量

系统状态中的 `recording_configured` 只统计已启用且配置为服务器录像的摄像机，不证明 MediaMTX 正在持续写盘。媒体服务状态、流就绪、录像文件增长和磁盘错误需要分别观测。

管理页的录像摄像头选择限定为当前实例的服务器录像设备。实例或设备列表变化时，当前选择会指向仍可用的摄像头；重新查询录像后，播放选择会清空，避免把上一查询的片段显示为本次结果。

管理页“日志”使用服务器操作系统的本地日期。打开页面时从受保护的 `GET /api/v1/logs/calendar`
取得服务器当天；单日查询使用 `date=YYYY-MM-DD`，范围查询使用 `start_date=YYYY-MM-DD&end_date=YYYY-MM-DD`，
包含起止当天且不与 `date` 混用。事件、媒体协调操作和业务审计每页最多 100 条，
返回 `next_cursor`，游标绑定日期范围和记录类型；同一时间戳以记录 ID 排序，不遗漏后续页。
页面保留当前页及已访问的游标栈，支持首页、上一页和下一页；上一页不是服务端 `previous_cursor` 字段。每组响应上限 8 MiB，服务端响应等待最多 5 秒；每条历史 SQLite 连接最多等待 2 秒，查询执行在 3 秒进度截止时中断。全局最多 4 个查询，连接及响应 body 结束前持续占用名额，至少 6 个连接留给命令及状态。

新历史写入和媒体操作在同一写事务中检查持久预算：所有事件、审计、操作和 outbox 合计最多 100 万行，每摄像头最多 10 万行且记录内容、索引元数据计费与回执预留最多 2 GiB，数据库、WAL 与已接受操作后续回执/审计预留合计最多 8 GiB；新增操作还要求可用空间保留 1 GiB 加预留量。达到任一边界时返回明确的 `history_storage_capacity`，不接受新操作、不删除已有事实。已有警报确认、PTZ 回执、操作完成及人工结果确认使用预留空间继续处理。容量上限是拒绝条件，不是实测性能保证，也不等同于真实磁盘已满。
服务端按该日期的本地起点到次日本地起点
换算为 UTC 半开区间，夏令时切换日也按实际时长处理。日志行显示服务器当地时间及该条记录
对应的 UTC 偏移；日期归属以记录创建时间为准。服务端时区配置或系统时钟变化后，刷新日志页
可取得新的服务器当天。`GET /api/v1/events` 保留实时监控快照用途，日志读取使用
`GET /api/v1/events/logs`。需要核对较早创建的 Unknown/Failed 媒体操作时，选择其创建日期；
媒体调和与状态判断直接读取持久化状态，不以当前日志筛选结果为依据。

## 管理员接口

控制面认证只接受以下三个路径：

- `POST /api/v1/auth/login`；
- `GET /api/v1/auth/session`；
- `POST /api/v1/auth/logout`。

login/session 成功响应必须严格是
`{authenticated:true,user_id,username,role:"admin",csrf_token}`，登录请求必须严格为
`{username,password}`；额外字段、已删除的 email 字段、其他 `role` 值和合同外路径都视为
合同错误。wire 中固定的 `role:"admin"` 仅用于跨产品响应一致性，不对应数据库列。实例授权码与
Client token 是独立的设备数据面凭据，与 Administrator 身份无关。

`BOOTSTRAP_ADMIN_USERNAME`、管理 API 和内置 Web 是 Server 范围。Client 保管的设备账号、MediaMTX internal
auth body、媒体 JWT、WHEP/HLS 播放与录像状态不使用 Administrator username；运维轮换管理 username/
密码时不能同步改实例授权码或媒体 key，反之亦然。

## 公共接口标识

当前版本只使用 `.state-instance.lock`、`.state-maintenance.lock`、`.state-maintenance-pending.json` 和 `.state-atomic-` 临时文件前缀。服务身份头为 `x-service`，健康状态中的公共源码修订字段为 `common_revision`。管理会话采用 `__Host-admin-xcos-session`，显式开发模式采用 `admin-xcos-session`；生产 Cookie 的 Secure、HttpOnly、SameSite、Path 和 CSRF 约束继续生效。资源清单格式为 `web-assets-v1`，公共数据库内部表及索引采用 `_common_` 前缀。
