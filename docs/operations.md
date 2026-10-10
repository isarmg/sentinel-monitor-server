# 配置与日常运维

首次安装见[安装指南](getting-started.md)，环境变量和端口见[配置参考](configuration.md)。默认部署由版本树内的 `xcosctl` 管理。

## 状态、启动与停止

```sh
sudo /opt/isarmg/xcos/releases/1.0.0/deploy/xcosctl status
sudo /opt/isarmg/xcos/releases/1.0.0/deploy/xcosctl start
sudo /opt/isarmg/xcos/releases/1.0.0/deploy/xcosctl stop
```

按需要执行相应命令。`status` 校验进程身份并查询实际 readiness，停服或未就绪时返回非零退出码。
应用结构化日志在 `/var/lib/isarmg/xcos/db/logs`；启动器和 MediaMTX 输出在外层 `logs` 目录。
平时关注画面、录像增长、磁盘容量、TLS 到期、协调操作和审计积压。

## Doctor 检查

默认启动器以 root 管理私有环境文件。进入受控 root Bash 后读取同一配置：

```bash
sudo bash
set -a
source /etc/isarmg/xcos.env
set +a
/opt/isarmg/xcos/releases/1.0.0/bin/xcos doctor --offline
/opt/isarmg/xcos/releases/1.0.0/bin/xcos doctor
exit
```

`--offline` 跳过在线 HTTP 探针，仍检查 Schema、SQLite、回滚写探针、录像目录、凭据解密和 MediaMTX 文件。
在线检查增加应用和媒体 readiness，并核对 MediaMTX 实际 `recordPath`。
使用 systemd 服务用户部署时，以实际服务用户和对应配置准备相同参数；不要将秘密复制到命令行或日志。

## systemd 运行方式

仓库 `deploy/xcos.service` 和 `deploy/xcos-mediamtx.service` 是可审阅的 Linux systemd 示例，需要运维创建 `xcos` 账户、安装审核过的完整只读版本树与受控 `current` 指针后使用。此方案与 `xcosctl` 后台启动方式二选一。该方案需在目标 systemd 主机验收实际服务启动、依赖关系和就绪状态。

把 `config/xcos.json.example` 配置为 `/etc/isarmg/xcos/config.json`，将私有父目录设为服务用户拥有的 `0700`，文件设为 `0600`；替换全部 secret/公网地址占位符，保持 credentials_key 与当前数据一致。`release_root` 固定为受控 `current`，同一配置来源不能再填写 `mediamtx_config`、`mediamtx_contract`、`mediamtx_binary`；它们由经验证的不可变 bundle 派生。`run --release-root` 明确覆盖配置来源的根选择。应用配置不得混入 companion 的 `MTX_*` 字段；将审阅过的 `config/mediamtx.env.example` 单独存为 `0600` 的 `mediamtx.env`，TLS key 同样只允许服务用户访问。

首次配置完成后，创建私有数据、录像与 runtime 目录，以服务用户交互执行一次核心 `--config ... init --username admin`（管理员密码从 stdin 读取），再 `config validate`。systemd 的 `ExecStartPre` 只校验，`ExecStart` 只运行，不在普通启动自动初始化。已有当前数据必须只读校验，不重置管理员。

```bash
/opt/isarmg/xcos/current/bin/xcos \
  --config /etc/isarmg/xcos/config.json config validate
/opt/isarmg/xcos/current/bin/xcos \
  --config /etc/isarmg/xcos/config.json run \
  --release-root /opt/isarmg/xcos/current
```

安装两个 unit 后以 `xcos.service` 管理部署。主 unit 的 Requires/After 确认 companion 启动依赖；companion 的 PartOf 使停服/重启同时停止其进程，再从 `current` 取得相同已验证版本。所有长驻进程以前台模式运行，由 systemd 直接跟踪。程序、companion、immutable YAML/lock 和 Web 清单都属于受验证的不可变发行 bundle，不列为可变 `state_paths`；数据库、录像、私有JSON和其他业务状态由只读state-contract与config validate报告。

## 文件布局

```text
/opt/isarmg/xcos/releases/1.0.0/
├─ RELEASE-MANIFEST
├─ bin/{xcos,mediamtx}
├─ share/web-assets.json
├─ config/{mediamtx.yml,mediamtx.lock}
└─ deploy/{xcosctl,common.sh,.bootstrap-action.sh,.start-action.sh,.status-action.sh,.stop-action.sh}

/etc/isarmg/xcos.env
/var/lib/isarmg/xcos/{db,recordings,logs}
/run/isarmg/xcos/{operations.lock,.state-maintenance.lock,.state-instance.lock,app.pid,mediamtx.lock,mediamtx.pid}

源码 `deploy/Caddyfile` -> 主机受管的 Caddy 配置
```

版本树 root-owned、只读且每个输入为物理路径。核心程序允许唯一受控部署指针 `/opt/isarmg/xcos/current`，该单跳链接仅能指向其所属安装目录 `releases/` 内的当前完整版本树，并继续核对全部 manifest、binary、companion 和静态清单；其他 alias 仍拒绝。原 `xcosctl` 使用版本树物理路径。下面的 systemd 方案使用核心 `current/bin` 入口与独立私有 JSON；程序始终验证指针所指向的完整物理发行树。

## 录像目录

`RECORDINGS_DIR`（JSON `recordings_directory`）是权威目录。它应为服务用户拥有的物理 `0700` 目录。
默认启动器由此设置 MediaMTX 的 `MTX_PATHDEFAULTS_RECORDPATH`；systemd 的独立媒体环境文件需填写同一个目录。
受管发行模板允许启动器设置此值，外部或开发 YAML 则需让实际 `recordPath` 与配置一致。在线 Doctor 检查最终生效值。

## 安全事件

先限制受影响的公网和摄像头网络入口，保存时间线、操作记录、日志与程序身份，再根据暴露范围处理对应凭据。
管理员密码、Client 授权码、JWT 与摄像头密码用途不同；授权码加密密钥的变更需同时考虑已有密文。
使用私密漏洞渠道提供脱敏证据，数据库、录像、完整 RTSP 地址和密钥留在受控环境。

下一步：[按症状排查](troubleshooting.md)，或查阅[运行状态参考](runtime-reference.md)。
