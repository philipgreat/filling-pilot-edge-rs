# filling-pilot-edge-rs 修复问题日志

> 记录 Rust 重写 filling-pilot-edge 过程中遇到并修复的所有问题。

---

## 1. 配置文件解析错误 — camelCase 字段映射

**日期**: 项目初期  
**提交**: `de5704b`  
**状态**: ✅ 已修复

### 问题
`serverConf` JSON 使用 camelCase（如 `serverAddress`），Rust struct 默认 snake_case，导致字段解析失败，所有配置值为默认值。

### 根因
`Context` struct 直接用 `#[serde(rename_all = "camelCase")]`，但同时包含 `id` 文件的字段，导致 rename 范围过大且 merge 逻辑中用了脆弱的默认值比较。

### 修复
- 拆分 `ServerConf` 为独立 struct，单独加 `#[serde(rename_all = "camelCase")]`
- `Context` 保持 snake_case（仅用于 `id` 文件）
- 去掉 merge 中的默认值比较，改为直接赋值

---

## 2. ContextError 缺少文件路径信息

**日期**: 项目初期  
**提交**: `3906928`  
**状态**: ✅ 已修复

### 问题
配置加载失败时错误信息只有 "missing field xxx"，不显示文件路径和工作目录，难以排查。

### 修复
- `ContextError` 改用 `thiserror` 定义枚举
- 每个错误变体包含 `cwd`、`id_path` 等上下文
- 错误 Display 输出完整文件路径和工作目录

---

## 3. TPKT 协议解析 Buffer 偏移 Bug

**日期**: 2026-04-14  
**提交**: `c7db26c`（通过升级 s7-connector 到 0.1.1）  
**状态**: ✅ 已修复

### 问题
TPKT 协议解析存在 buffer 偏移 bug：`tpkt_len` 包含了 4 字节 TPKT header，但代码跳过 header 后又多读了 4 字节，导致后续 COTP/S7 数据全部错位。

### 根因
s7-connector crate 0.1.0 中 TPKT 解析逻辑错误。

### 修复
- 在 [s7connector-rs](https://github.com/philipgreat/s7connector-rs) 修复 TPKT/COTP 协议解析
- 发布 s7-connector 0.1.1 到 crates.io
- filling-pilot-edge-rs 升级依赖：`c7db26c`

---

## 4. 心跳不处理 plcInfo 命令

**日期**: 2026-04-20~21  
**提交**: `9c5a375`  
**状态**: ✅ 已修复

### 问题
Java 版中 `register()` 和 `heartBeat()` 都使用 `ServerCommandObserver`，会分发 `plcInfo` 命令。Rust 版只在 `register()` 流中处理 `plcInfo`，心跳流中忽略了这个命令。

### 影响
PLC 配置只在注册时获取一次，心跳中的配置变更无法更新。

### 修复
心跳响应也调用 `handle_command()` 处理服务器命令（包括 plcInfo），与 Java 行为一致。

---

## 5. plcInfo handler 跳过配置未变更的心跳

**日期**: 2026-04-21 ~06:00  
**提交**: `0f0a059`  
**状态**: ✅ 已修复

### 问题
plcInfo handler 有 early return 逻辑：`if *last == cmd.detail { return false; }`，配置没变就跳过。但 Java 版**每次心跳都发送 plcInfo 响应**，因为服务器用 TTL 判定 PLC 在线状态（60s），必须持续续期。

### 修复
去掉 early return，每次心跳都：
1. 解析 PLC 列表
2. TCP 测试每个 PLC
3. 发送 `send_plc_response(plcId, "plcInfo", "ok")`
4. 最终发送 `send_plc_response("plcInfo", "plcInfo", "ok")`

---

## 6. MutexGuard 跨 .await 导致编译错误

**日期**: 2026-04-21 ~06:00  
**提交**: `0f0a059`  
**状态**: ✅ 已修复

### 问题
`std::sync::MutexGuard<'_, String>` 跨 `.await` 点不是 `Send`，编译报错。

```
error: future cannot be sent between threads safely
`std::sync::MutexGuard<'_, String>` cannot be sent between threads safely
```

### 修复
将 mutex lock 提取到独立 `{}` 块中，确保 guard 在 `.await` 前释放：

```rust
// 修复前（错误）
let last = self.last_plc_detail.lock().unwrap();
if *last != cmd.detail {
    // ... .await ...  ← guard 仍然存活
}

// 修复后
let config_changed = {
    let last = self.last_plc_detail.lock().unwrap();
    *last != cmd.detail
};  // ← guard 在此处释放
// .await 安全
```

---

## 7. PLC 状态上报消息类型错误

**日期**: 2026-04-21 ~06:00  
**提交**: `0f0a059`  
**状态**: ✅ 已修复

### 问题
`send_plc_status` 发送的消息类型是 `plc_connected` / `plc_disconnected`，但服务器端**只处理**以下类型：`plcInfo`, `status`, `monitorStatus`, `report`, `clientInfo`, `config`, `ReportSubmitted`。

`plc_connected` 不在处理列表中，服务器直接忽略。

### 修复
plcInfo handler 中改为发送 `"plcInfo"` / `"plcInfoFail"` 类型，与 Java `PlcInfo.handle()` 一致。

---

## 8. ECDSA 签名使用了错误的椭圆曲线

**日期**: 2026-04-21 ~06:37  
**提交**: `b21e917`  
**状态**: ✅ 已修复

### 问题
所有 PlcResponse 签名为空字符串。运行日志：

```
Failed to parse private key: pkcs8_der=public key error: 
unknown/unsupported algorithm OID: 1.3.132.0.10
```

### 根因
- 服务器证书使用的私钥是 **P-256 (secp256r1 / prime256v1)** 曲线
- OID = `1.2.840.10045.3.1.7`
- 代码使用了 `k256` crate（**secp256k1** 曲线，OID = `1.3.132.0.10`），曲线完全错误
- `k256` 无法解析 P-256 格式的私钥，导致签名函数静默失败返回空

### 修复
1. `Cargo.toml`: `k256` → `p256`（同样 v0.13，features 不变）
2. `src/grpc/cloud.rs`: 所有 `use k256::` → `use p256::`

### 验证
- `from_pkcs8_der OK!` — 每次成功解析私钥
- `sign: 5d1c52c0...` — 128 字符 hex 签名
- 心跳每秒发送，所有 plcInfo 响应带有效签名

---

## 9. PlcResponse 的 plc_id 发送了 IP:port 而非 PLC 配置 ID

**日期**: 2026-04-21 ~06:47  
**提交**: `b21e917`  
**状态**: ✅ 已修复

### 问题
`plc_id` 字段发送的是 `127.0.0.1:102`，而服务器期望 PLC 配置中的 `id`（如 `PP000093`）。

### 根因
Java 参考（`PlcInfo.java:51`）：
```java
String id = (String) plc.get("id");
session.sendPlcResponse(id, "plcInfo", "ok");
```
Rust 用了 `format!("{}:{}", plc.ip, plc.port)` 代替 PLC id。

### 修复
1. `PlcConfig` struct 增加 `id: String` 字段
2. `parse_plc_list()` 解析 `plc.get("id")`
3. plcInfo handler 发送 `&plc.id` 替代 `&format!("{}:{}", plc.ip, plc.port)`

### 验证
- `plc_id: PP000093` ✅（之前 `127.0.0.1:102`）
- 服务器确认 PLC 显示在线

### 注意
`send_plc_status`（独立状态循环，每 10 秒）仍用 IP:port，非关键路径。

---

## 10. PLC 每秒 TCP CONNECT/DISCONNECT（非 bug，已记录）

**日期**: 2026-04-21 ~06:55  
**状态**: ⚠️ 预期行为，已文档化

### 现象
模拟器日志每秒出现 `CONNECT → CLOSE (EOF) → DISCONNECT`。

### 原因
每条 `plcInfo` 心跳都会调用 `test_tcp_connection()`，这是原始 TCP socket 连通性测试：
```rust
async fn test_tcp_connection(&self, host: &str, port: u16) -> bool {
    match tokio::time::timeout(3s, TcpStream::connect(addr)).await {
        Ok(Ok(stream)) => { /* connected, drop immediately */ }
    }
}
```
TCP 连上就断开，仅判断 PLC 是否可达，不涉及 S7 协议。与 Java 版 `Socket.connect(3000ms)` 行为一致。

### 处理
在 README.md 中文档化此行为（`1764cd5`），不做代码改动。

---

## 附录：两套 PLC 连接检测机制

| 机制 | 频率 | 方式 | 用途 | 来源 |
|------|------|------|------|------|
| `test_tcp_connection` | 每心跳 (~1s) | 原始 TCP socket | 心跳中报告 PLC 在线状态 | plcInfo handler |
| `check_all_connections` | 每 10s | S7 `read_db(1,0,1)` | 本地状态日志 + 云端 plcStatus | 独立后台任务 |
