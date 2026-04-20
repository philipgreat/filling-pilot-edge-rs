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

## 已知行为

### PLC 每秒 TCP CONNECT/DISCONNECT

每条 `plcInfo` 心跳都会对每个 PLC 做 TCP 层连通性测试（`test_tcp_connection`），这是**预期行为**，与 Java 版一致。测试流程：TCP 连接 → 立即断开，仅用于判断 PLC 是否可达，不涉及 S7 协议握手。

模拟器日志中每秒出现的 `CONNECT → CLOSE (EOF) → DISCONNECT` 即为此测试，非异常。

此外 `check_all_connections`（每 10 秒）会做 S7 协议层的 `read_db` 验证，两套检测独立运行。`read_db` 失败时会从连接池移除该连接，下次重新建立。

### 两套连接检测机制

| 机制 | 频率 | 方式 | 用途 |
|------|------|------|------|
| `test_tcp_connection` | 每心跳 (~1s) | 原始 TCP socket | 心跳中报告 PLC 在线状态 |
| `check_all_connections` | 每 10s | S7 `read_db(1,0,1)` | 本地状态日志 + 云端 plcStatus 上报 |

## 依赖

- [s7-connector](https://crates.io/crates/s7-connector) - S7 协议通信库

## 对应 Java 版本

本项目是 [filling-pilot-edge](https://github.com/philipgreat/filling-pilot-edge) 的 Rust 重写。
