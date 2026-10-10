# 安装并播放第一路视频

准备一台 Linux AMD64 GNU 主机、HTTPS 域名、RTSPS 证书，以及位于摄像头网络的 xcoc 客户端。目标是登录管理页并播放一台测试摄像头。

## 1. 选择同一源码的程序与文档

[公开下载页](https://github.com/isarmg/xcos/releases)上的 v1.0.0 归档早于当前 `xcos-db-v2` 实现。使用下载包时按包内说明；使用当前源码时按[开发指南](development.md)构建，并满足正式打包的干净源码与精确 annotated tag 条件。

以下安装流程面向通过完整验证的同版归档，使用默认 `/opt/isarmg/xcos/releases/1.0.0` 路径。安装目标须为全新目录，归档不会覆盖已存在的发行树。

## 2. 校验并安装归档

在 Linux 主机中，把归档与同版 `SHA256SUMS` 放在同一目录，执行：

```sh
sha256sum --check SHA256SUMS
sudo tar -xzf xcos-1.0.0-x86_64-unknown-linux-gnu.tar.gz \
  -C / --keep-old-files --no-overwrite-dir opt
sudo /opt/isarmg/xcos/releases/1.0.0/deploy/xcosctl bootstrap
sudoedit /etc/isarmg/xcos.env
```

校验应显示归档为 `OK`。bootstrap 创建私有配置与目录，暂不启动服务。

## 3. 配置入口和凭据

在配置文件中确认管理员密码、独立的 JWT Secret 与 Credential Key，并填写：

- `PUBLIC_RTSP_PUBLISH_BASE_URL`：客户端可达的 `rtsps://主机名:8322`。
- `MEDIAMTX_RTSP_CERT`、`MEDIAMTX_RTSP_KEY`：与该主机名匹配且受客户端信任的证书和私钥。
- `MEDIA_PUBLIC_HOSTS`：浏览器可达的 DNS/IP。

按[配置与网络参考](configuration.md)将仓库 `deploy/Caddyfile` 的站点块纳入主机 HTTPS 配置。控制面 8080 和 MediaMTX 管理端口保留在受控网络；按实际客户端与浏览器网络开放 RTSPS 和 WebRTC 所需端口。

授权码加密依赖 `CREDENTIALS_KEY`。将该密钥持久保存在受控秘密存储中，运行期间保持与当前数据匹配；丢失或替换会使已有密文无法读取。

## 4. 初始化并启动

```sh
sudo /opt/isarmg/xcos/releases/1.0.0/deploy/xcosctl bootstrap --confirm-config
sudo /opt/isarmg/xcos/releases/1.0.0/deploy/xcosctl start
sudo /opt/isarmg/xcos/releases/1.0.0/deploy/xcosctl status
```

确认步骤显式创建当前数据库与管理员，并只读校验配置；成功后移除临时管理员密码。已有数据库只校验。`status` 应报告已核实身份且就绪的服务。

## 5. 登录并连接摄像头

打开实际 HTTPS 域名，使用刚配置的管理员登录，按[日常使用](usage.md)创建实例并在 xcoc 配对。
成功标准是管理页收到新快照、主码流可播放，选择录像后出现新片段。

停止默认启动器管理的服务：

```sh
sudo /opt/isarmg/xcos/releases/1.0.0/deploy/xcosctl stop
```

希望由 systemd 常驻管理时，使用[systemd 方案](operations.md#systemd-运行方式)，同一部署只选择一种进程管理方式。
