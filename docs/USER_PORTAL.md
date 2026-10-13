# 用户中心与原版用户 Token

用户中心入口为 `/console`，注册页为 `/register`，管理员入口为 `/admin`。用户中心负责网站注册、登录和账户与密钥的关联；模型请求直接使用原版用户 Token 功能。

## 邮箱验证与找回密码

schema v5 增加邮箱状态和一次性验证凭据。已有账户保持已验证状态。配置 SMTP 后，新注册账户默认先验证邮箱再创建原版 API Key；可显式设置 `ABV_PORTAL_REQUIRE_EMAIL_VERIFICATION=false` 允许先使用密钥，邮箱验证仍可进行。未配置 SMTP 时保持原注册方式，找回密码页面明确提示联系管理员；若强制验证却未配置 SMTP，初始化失败，避免注册后无法使用。

SMTP 环境变量：`ABV_SMTP_HOST`、`ABV_SMTP_PORT`、`ABV_SMTP_FROM`、`ABV_SMTP_USERNAME`、`ABV_SMTP_PASSWORD`、`ABV_SMTP_SECURITY`。安全模式默认为 `starttls`（587），也支持 `tls`（465）；证书必须有效，STARTTLS 不允许降级。`local` 模式仅允许 `127.0.0.1` 或 `::1`，用于本地收信验收，端口需明确指定。SMTP 凭据仅保存在服务端环境配置中，不能写入前端配置或公开仓库。

- 注册后自动请求验证邮件；登录后可重发及粘贴验证码。`POST /portal/api/auth/verification` 和 `/auth/verify` 均要求当前会话、同源与 CSRF。
- 忘记密码：`POST /portal/api/auth/forgot` 接收邮箱，存在和不存在的账户返回相同信息，发信在后台执行；`POST /portal/api/auth/reset` 接收邮件验证码和新密码。
- 验证码为 256 位随机值，数据库仅存 SHA-256 摘要，用途隔离、20 分钟过期、仅用一次；重发使上一个同用途验证码失效。发信失败会注销该验证码，日志不包含地址、SMTP 密码或验证码。
- 找回密码使用 Argon2 更新密码并注销该账户全部登录会话及其他验证码；API Key 保留，用户可登录后另行重置。未授权、错误用途、过期或重复验证码均不能修改账户。
- 发信和验证码提交均有频率限制；单次 SMTP 命令超时 15 秒、整体发送超时 20 秒。本地 SMTP 端到端回归使用保留域名 `example.invalid`，不向真实用户发邮件。

邮件实现参考 [lettre SMTP 文档](https://docs.rs/lettre/latest/lettre/transport/smtp/struct.AsyncSmtpTransport.html)。部署时还需配置真实发件服务并验收投递，代码及本地收信通过不能代替实际投递验收。

## 人工充值与退款

充值必须填写实际流水号；同一流水号和相同金额、账户的重试不会重复入账。用户和管理员均可查看最近 100 条充值与退款记录。在线支付继续关闭。

用户在充值记录中提交退款申请：`POST /portal/api/refunds`，参数为 `credit_reference`、`amount_cents`、`reason`，要求会话、同源和 CSRF。金额按整数分输入；申请总额不能超过该笔充值扣除未取消退款后的金额，也不能申请他人的充值。

管理员在用户详情审核，通过 `POST /api/portal/refunds/:id` 执行 `approve`、`pay`、`cancel`，沿用严格管理员鉴权。状态流程：

1. `requested`：待审核，尚未移动余额。
2. `approved`：事务中核对无在途调用或待核对费用、余额足够后扣除退款金额；重复审核不会再扣。
3. `paid`：管理员完成线下打款后填写唯一的 `payment_reference`，登记已完成；该接口本身不转账。
4. `cancelled`：待审核可取消；已审核未打款可取消并恢复余额，重复取消不会重复返还。已完成退款不能取消。

审核、扣除或返还余额、状态更新和审计在同一 SQLite 事务中完成，防止并发操作重复改变余额。管理员充值后应先核对实际入账，退款标记完成前应先核对真实线下流水。管理员流水查看接口：`GET /api/portal/users/:id/history`。

## 请求编号与故障追踪

计费请求的账本 ID、监控日志 ID 和响应 `X-Request-ID` 相同。管理员监控详情展示各次上游尝试的实际模型、账号 ID、端点主机、HTTP 状态和耗时；最多保存前 32 次并展示截断提示，端点只保留主机名。日志支持按请求 ID 搜索。旧日志无尝试数据时明确提示。

API 测试结果显示请求 ID。账号错误弹窗区分身份验证、令牌刷新、停用、限流及权限或地区限制，提供恢复建议、额度刷新与实际 API 复测入口。额度查询成功不等同于生成成功；复测后应以请求日志及最终响应为准。

## 模型调用和统计

- 注册账户通过原版 `user_token_db::create_token` 创建 `sk-*` Token，保存到原版 `user_tokens.db`，可在管理员“用户 Token”页面查看和管理。默认长期有效、不限制 IP、不设置访问时段。
- 账户 API 使用原版鉴权、`UserTokenIdentity`、协议适配、模型映射、上游账号选择、思考和上下文处理、端点回退与重试。
- 用量由原版监控中间件调用 `record_token_usage_and_ip` 记录。用户中心直接读取 `user_tokens.total_requests`、`total_tokens_used` 和 `token_usage_logs`，不重新估算、解析或计算 token。
- 模型请求保持原版模型、思考、协议与重试能力。存在人民币钱包的账户在调用前检查余额、完成后按权威用量结算；无钱包的旧账户保留原有行为。没有额外模型白名单、输出预算或额度预留。
- 密钥重置通过原版创建与停用接口实现。重置保留管理员配置的到期时间、IP、访问时段及启用状态。原密钥的记录仍保留在原版管理功能中；账户页展示当前密钥的累计统计。停用网站账户同步停用其当前原版 Token。
- 登录后可查看并复制当前原版密钥。管理员在原版界面停用或删除 Token 后，访问账户页不会自动重新创建或启用它。

## 历史数据与迁移

`portal.db` schema v3 新增 `native_token_links`，仅保存网站用户 ID 与原版 Token ID 的关联。升级时为已有账户创建关联的原版 Token；已有非网站用户 Token 保持原样。

旧的 `sk-portal-*` 密钥失效。用户登录后点击“查看并复制”，将新的 `sk-*` 密钥更新到客户端。密钥鉴权直接进入原版链路，不再识别旧的独立计费密钥。

schema v4 新增人民币钱包与调用账本：新账户创建零余额钱包；旧账户在管理员首次充值后也会启用人民币计费。旧 Token 兑换、注册赠送、订单和流水保留为历史记录，不参与人民币计费。在线支付未开启，旧充值和支付通知接口仍返回 HTTP 410。

## 人民币计费与核对

- 每百万 token：普通输入 ¥0.60、缓存输入 ¥0.06、输出（含思考）¥3.00。总输入包含缓存，先拆分再计费；金额采用 nano-yuan 整数，1 元 = 1,000,000,000 nano-yuan。
- 调用前检查余额大于零，在途并发请求按实际用量结算，余额可能变为负数，之后的新调用被 402 拦截。当前不预留额度，不保证在余额耗尽的一瞬间截断响应。
- 仅成功且有权威用量的调用结算。HTTP 失败、上游流中断（记录 502）和客户端取消（记录 499）不收费；已发出的响应头不改写。已结算 / 已释放请求重复提交不会再次扣费。
- 成功但缺少权威用量、重启遗留 pending 请求进入 review，暂停该用户的新计费调用，等待管理员核对。
- 用户页显示最近 50 笔请求的编号、模型、普通输入 / 缓存 / 输出、费用分项、状态和结算时间。未知用量显示“—”。余额和费用属于网站账户，Key 轮换不会清空，已鉴权的在途旧 Key 请求仍归属原账户。
- 管理员“注册用户 → 余额 / 用量”查看明细并核对 review 记录；以权威记录填写总输入（含缓存）、缓存、输出和 HTTP 状态。确认失败可用失败状态释放，不猜填成功用量。
- `POST /api/portal/users/:id/credits` 接受 `amount_cents` 与唯一 `reference`；重复的同一流水只入账一次。`POST /api/portal/requests/:id/settle` 核对费用。两者均在严格管理员鉴权之后，普通用户不可调用。

升级前保留镜像、配置以及 `portal.db` 与 `user_tokens.db` 的一致快照。回滚需恢复匹配版本；旧版程序不能直接打开较新 schema。恢复演练使用独立数据目录，不覆盖现有用户、余额和流水。

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

`Portal billing regression` 工作流对 beta / main 的相关变更和 PR 运行原生 Key、计费、幂等、并发与重启恢复测试。测试使用独立数据，不调用真实上游或消耗生产余额。完整流式中间件的编译由原有应用 CI 验证；线上取消 / 断流验收需另行记录，不能用账本测试代替。
