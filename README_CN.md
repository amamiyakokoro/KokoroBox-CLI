# KokoroBox-Shell

Kokoro 的 GNU/Linux 命令列客戶端。可從 SSH 登入、切換訂閱設定，用 Mihomo 載入 Kokoro 規則，並為本機提供 HTTP／SOCKS 或透明代理。[English](README.md)

Rust 管理 Kokoro 登入與訂閱、Mihomo 啟停，並透過 iproute2 和 nftables 設定本機透明代理。互動欄位選擇器與 Mihomo YAML 設定的驗證和重載由 Python 處理。

## 安裝

在 GNU/Linux 上使用 Rust／Cargo 1.85 以上建置原生命令列程式。Debian、Ubuntu 可執行以下命令。

```sh
sudo apt-get install bash python3 python3-yaml curl ca-certificates iproute2 util-linux passwd build-essential pkg-config libcurl4-openssl-dev libssl-dev
git clone --depth 1 --single-branch -b dev https://github.com/amamiyakokoro/KokoroBox-Shell.git
cd KokoroBox-Shell
cargo build --release --locked
sudo env KOKOROBOX_CLI_BIN="$PWD/target/release/kokorobox" bash install.sh
```

可透過 `KOKOROBOX_CLI_BIN` 安裝適用目標 GNU/Linux 架構的預建檔，目標主機無須安裝 Rust；未指定時，安裝器會用 Cargo 從原始碼建置。

安裝器會依 x86_64、aarch64、ARMv5–v7 或 MIPS 架構下載指定版本的 Mihomo 官方發行檔，並比對倉庫記錄的 SHA-256。離線安裝可同時指定預先編譯的 CLI 與對應架構的官方 `.gz` 檔，以下以 ARM64 為例。

```sh
sudo env KOKOROBOX_CLI_BIN=/path/to/kokorobox \
    KOKOROBOX_MIHOMO_ARCHIVE=/path/to/mihomo-linux-arm64-v1.19.34.gz bash install.sh
```

首次互動安裝會先選擇簡中、繁中或英文，再開啟主選單。日後可在 **3 → b** 更換語言。非互動安裝可指定 `KOKOROBOX_LANG=chs|cht|en`，也可留待第一次開啟選單時選擇。

## 登入、切換設定與同步

```sh
sudo kokorobox login
sudo kokorobox select
sudo kokorobox start
sudo kokorobox status
```

在另一台裝置開啟登入網址並確認代碼。`select` 會列出帳號能選擇的方案、協定、ISP、連線模式、規則來源與最終路由。按 Enter 沿用預設，輸入 `q` 取消。想直接使用伺服器預設值，登入後執行 `sync` 即可。也可以執行 `subscribe`，在隱藏輸入提示貼上 Kokoro 產生的外部 Mihomo 訂閱連結，或從標準輸入傳入。

首次選擇帳號訂閱後，`select` 會顯示摘要，輸入項目編號即可單獨修改，`s` 套用、`q` 取消。`subscribe` 切到外部連結，`select` 切到已登入帳號。登入後，手動與排程同步仍沿用選定來源；更換帳號時需用 `select` 明確切換。

### 首次帳戶訂閱的預設值

首次登入後可執行 `sudo kokorobox sync` 採用帳戶預設值。若目前使用外部訂閱，可透過 `sudo kokorobox select` 切回帳戶。CLI 依帳戶可用選項決定預設值，目前 Kokoro 提供以下設定。

| 設定 | 預設值 |
|---|---|
| 核心與訂閱格式 | Mihomo |
| 方案與 ISP | 帳戶第一個可用方案，ISP 自動 |
| 協定與連線模式 | VMess，中繼 |
| 規則來源與最終路由 | 原站，未匹配的流量走代理 |
| 規則集自動更新 | 開啟 |
| 訂閱宣告的更新間隔 | 開啟，1 小時 |

若希望主機每小時實際同步，執行 `sudo kokorobox auto-sync on` 開啟本機排程。首次安裝僅監聽本機，透明代理預設關閉。

本機 HTTP／SOCKS 預設位址為 **127.0.0.1:7890**。`select`、`sync` 會用 Mihomo 驗證實際執行設定，執行中的服務會直接套用，內容相同時略過重載。驗證失敗會保留舊訂閱。節點群組、規則順序、引用的規則集與最終 `MATCH` 依照 Kokoro 訂閱。可在 Kokoro 網頁修改個人規則後執行 `sync`。登入憑證與訂閱只允許 root 讀寫。`logout` 會撤銷本機工作階段並保留已下載的設定。

## 本機透明代理

先安裝 `nftables` 並確認 Linux 提供 TUN。容器需允許 `/dev/net/tun` 與 `CAP_NET_ADMIN`。

```sh
sudo kokorobox transparent on
sudo kokorobox transparent off
```

切換會記住模式，服務執行中時會重新啟動。本機發出的 TCP／UDP 經 nftables 的 output 鏈與 IPv4／IPv6 策略路由進入 Mihomo 的 system TUN。進入 TUN 的 DNS 查詢由 Mihomo 處理。回環、鏈路本地流量、外部連入連線的回應，以及核心本身的連線沿用系統路由。客戶端不會開啟 IP 轉送。透明模式使用介面名稱 `kbox0`、路由表及優先順序 18992。

## 管理

| 子命令 | 用途 |
|---|---|
| `whoami`、`logout` | 查看帳號、登出 |
| `status` | 查看服務狀態、代理位址、透明模式及訂閱設定 |
| `start`、`stop`、`restart` | 啟動、停止及重新啟動服務 |
| `logs`、`test` | 檢視近期核心日誌、實測本機代理 HTTPS 連線 |
| `auto-sync on\|off\|show` | 每小時同步訂閱、停用或查看排程 |
| `uninstall` | 確認後移除安裝與本機資料 |

使用 `sudo kokorobox 子命令` 執行，或直接執行 `sudo kokorobox` 開啟選單。自動同步需有執行中的 cron，服務停止時只更新設定。傳給 Kokoro 的訂閱更新間隔與本機排程分開設定。開機自啟可使用 `sudo systemctl enable kokorobox.service` 或 `sudo rc-update add kokorobox default`。沒有這兩種管理器的主機也可直接透過 CLI 啟停。

升級時拉取新版、重新建置 CLI 後執行安裝器。既有帳號、語言、訂閱選擇及定時同步排程會保留。舊版本機或混合接管會遷移為本機 TUN，升級前需備妥 nftables。原本僅代理區網的安裝改用回環代理。本機手動規則覆寫與面板選擇停止套用，既有 sing-box JSON 設定會保留供後續相容。

## 授權

本專案衍生自 [ShellCrash](https://github.com/juewuy/ShellCrash)，採用 [GPLv3](LICENSE.txt)。Mihomo 與隨附的規則資料沿用原授權。
