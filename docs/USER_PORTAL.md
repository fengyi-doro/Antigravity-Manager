# 用户中心与原版用户 Token

用户中心入口为 `/console`，注册页为 `/register`，管理员入口为 `/admin`。用户中心负责网站注册、登录和账户与密钥的关联；模型请求直接使用原版用户 Token 功能。

## 模型调用和统计

- 注册账户通过原版 `user_token_db::create_token` 创建 `sk-*` Token，保存到原版 `user_tokens.db`，可在管理员“用户 Token”页面查看和管理。默认长期有效、不限制 IP、不设置访问时段。
- 账户 API 使用原版鉴权、`UserTokenIdentity`、协议适配、模型映射、上游账号选择、思考和上下文处理、端点回退与重试。
- 用量由原版监控中间件调用 `record_token_usage_and_ip` 记录。用户中心直接读取 `user_tokens.total_requests`、`total_tokens_used` 和 `token_usage_logs`，不重新估算、解析或计算 token。
- 账户 API 没有额外的模型白名单、输出/思考预算、流式参数改写、请求预留、扣费或重试限制。模型与协议能力和原版用户 Token 一致。
- 密钥重置通过原版创建与停用接口实现。重置保留管理员配置的到期时间、IP、访问时段及启用状态。原密钥的记录仍保留在原版管理功能中；账户页展示当前密钥的累计统计。停用网站账户同步停用其当前原版 Token。
- 登录后可查看并复制当前原版密钥。管理员在原版界面停用或删除 Token 后，访问账户页不会自动重新创建或启用它。

## 历史数据与迁移

`portal.db` schema v3 新增 `native_token_links`，仅保存网站用户 ID 与原版 Token ID 的关联。升级时为已有账户创建关联的原版 Token；已有非网站用户 Token 保持原样。

旧的 `sk-portal-*` 密钥失效。用户登录后点击“查看并复制”，将新的 `sk-*` 密钥更新到客户端。密钥鉴权直接进入原版链路，不再识别旧的独立计费密钥。

独立充值、注册赠送、额度预留、扣费及人工结算已停用。历史余额、赠送、订单、请求和流水记录保留，用户页提供历史记录查看；这些记录不参与当前用量或访问权限判断。旧充值、支付通知及结算接口返回 HTTP 410。旧模型白名单配置保留在数据库中，但不再影响模型调用。

升级前同时备份 `portal.db` 与 `user_tokens.db`。回滚需要恢复匹配的镜像和两个数据库，不能让仅支持 v2 的旧程序直接打开 v3 数据库。

## 网站身份与配置

网站密码仍使用 Argon2id；会话使用 HttpOnly、SameSite=Strict Cookie，HTTPS 下设置 Secure。修改密钥及退出登录需要验证 Origin 和 CSRF token。网站账户凭据不能用于管理员接口。

- `ABV_PORTAL_PUBLIC_URL`：用户中心源站地址，生产使用 HTTPS。
- `ABV_PORTAL_API_BASE_URL`：账户页显示的 API 地址。
- `ABV_PORTAL_REGISTRATION_OPEN=false`：暂停注册。
- `ABV_PORTAL_TRUST_PROXY=true`：入口可靠覆盖客户端 IP 时，用于注册和登录限流。
- `WEB_PASSWORD` / `API_KEY`：原版管理员密码与主 API Key。

`ABV_PORTAL_ALLOWED_MODELS` 和 `ABV_EPAY_*` 不再参与运行逻辑。API 的鉴权模式、模型、请求大小和上游配置均使用原版设置。

## 验证

```bash
cargo test --manifest-path src-tauri/crates/portal/Cargo.toml -- --test-threads=1
npm run build
```

集成测试直接包含原版 `user_token_db.rs` 与生产适配器，验证原版密钥创建、鉴权、重置、停用、统计一致、账户隔离、CSRF、旧数据保留及计费接口停用。
