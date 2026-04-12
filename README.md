# Filling Pilot Edge - Rust

工业物联网边缘计算平台，用于西门子 S7 PLC 数据采集与云端通信。

## 项目结构

```
filling-pilot-edge-rs/
├── src/
│   ├── main.rs          # 入口
│   ├── lib.rs           # 库入口
│   ├── context.rs       # 配置管理
│   ├── error.rs        # 错误类型
│   ├── http.rs         # 本地 HTTP API
│   ├── s7.rs           # S7 PLC 通信
│   ├── codec/          # 数据块编解码
│   ├── grpc/           # gRPC 类型定义
│   └── processor/       # 命令处理器
├── proto/               # gRPC proto 定义
└── Cargo.toml
```

## 功能

- **S7 PLC 通信**：通过 TCP/IP 连接西门子 PLC，读写数据块
- **本地 HTTP API**：部署数据块配置、手动读写 PLC
- **命令处理器**：Read、Write、Status、Restart、Upgrade
- **数据块编解码**：支持 Boolean、Integer、Real、String、DateTime 等类型

## 配置

在运行目录创建两个配置文件：

### id
```json
{
    "id": "your-edge-node-id",
    "privateKey": "optional-private-key-for-signing"
}
```

### serverConf
```json
{
    "serverAddress": "192.168.0.1",
    "port": 9999,
    "heartBeat": 5000,
    "reportInterval": 5000,
    "statusInterval": 1000,
    "localPort": 22222
}
```

## 运行

```bash
cargo run
```

## HTTP API

- `GET /` - Web 管理界面
- `POST /deploy` - 部署数据块配置
- `GET /status` - 节点状态
- `POST /read` - 手动读取 PLC
- `POST /write` - 手动写入 PLC
- `GET /health` - 健康检查

## 依赖

- [s7-connector](https://crates.io/crates/s7-connector) - S7 协议通信库

## 对应 Java 版本

本项目是 [filling-pilot-edge](https://github.com/philipgreat/filling-pilot-edge) 的 Rust 重写。
