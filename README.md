# xcos

## 项目简要介绍

自托管的摄像头监控服务。Rust 控制面管理设备和录像，配套 MediaMTX 处理媒体接入与播放，摄像头凭据由独立的 xcoc 客户端保存。

## 项目功能

- 管理管理员、客户端实例与摄像头，查看设备和媒体状态
- 浏览器实时播放、云台控制、主/子码流及录像索引
- 短期媒体发布授权、服务端录像与媒体状态协调

## 适用平台

服务端仅支持 Linux AMD64 GNU（`x86_64-unknown-linux-gnu`）。生产环境需要 HTTPS 反向代理，以及客户端可访问、证书受信的 RTSPS 入口。

## 如何快速部署

从 [下载页](https://github.com/isarmg/xcos/releases) 取得同版 Linux 归档与 `SHA256SUMS`。以下用于全新安装，不能覆盖已有发行树；公开 v1.0.0 归档早于当前 `xcos-db-v2` 合同，应使用各自版本的部署配置。

```sh
sha256sum --check SHA256SUMS
sudo tar -xzf xcos-1.0.0-x86_64-unknown-linux-gnu.tar.gz -C / --keep-old-files --no-overwrite-dir opt
sudo /opt/isarmg/xcos/releases/1.0.0/deploy/xcosctl bootstrap
sudoedit /etc/isarmg/xcos.env
sudo /opt/isarmg/xcos/releases/1.0.0/deploy/xcosctl bootstrap --confirm-config
sudo /opt/isarmg/xcos/releases/1.0.0/deploy/xcosctl start
sudo /opt/isarmg/xcos/releases/1.0.0/deploy/xcosctl status
```

确认配置前，填写管理员密码、独立密钥、公开 RTSPS 地址和证书路径，并配置 HTTPS 网关。`bootstrap --confirm-config` 显式初始化；普通启动不建库。控制面和 MediaMTX 管理端口只向本机网关开放。

## 如何编译部署

在 Linux AMD64 上准备 Rust 1.99.0、Node.js 26.7.0、C 编译工具、Python 3、curl、OpenSSL 和 GNU 工具。正式打包要求干净源码，且同版本 annotated tag 精确指向 HEAD；不能把未发布源码当成已有标签的制品。

```sh
rustup target add --toolchain 1.99.0 x86_64-unknown-linux-gnu
output="$(mktemp -d /var/tmp/xcos-release.XXXXXXXX)"
bash scripts/package-release.sh "$output"
```

脚本下载并校验固定的 MediaMTX，构建内嵌 Web 的服务端，输出归档与校验文件。随后按上面的全新安装步骤部署生成的包。

[详细文档](docs/README.md)
