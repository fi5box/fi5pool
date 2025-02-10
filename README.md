# Fi5Pool

Fi5Pool is a simple, lightweight, and easy-to-use pool for mining ckb. It is designed to be easy to set up and use, and to be a good choice for small miners who want to mine ckb without having to worry about setting up and maintaining a complex pool infrastructure.

## Build Binary

```bash
cargo build --release
```

## Install Binary

```bash
cargo install --path .
```

## Build Docker Image

```bash
docker build -t fi5pool .
```

## Package Deb

```bash
./package.sh
```

## Usage

```bash
> fi5pool -h

Usage: fi5pool [OPTIONS]

Options:
  -c, --config <CONFIG_PATH>  [default: config.toml]
  -h, --help                  Print help
  -V, --version               Print version
```

```bash
> fi5pool -c config.toml

...
01-13 10:38:38.733  INFO ThreadId(01) fi5pool: Difficulty: 800000, difficulty_for_log: 0.8, target: 144740111546645244279463731260859884816587480832050705049321980009891412
01-13 10:38:38.733  INFO ThreadId(01) fi5pool: Server listening on: [::]:34255
01-13 10:38:38.739  INFO ThreadId(23) fi5pool::http: http listening on [::]:8888
```

### Configuration

```toml
# rpc listen port
rpc_listen_port = 34255
# http listen port
http_listen_port = 8888
# rpc url of the ckb node
ckb_rpc_url = "http://127.0.0.1:8114"
# difficulty of the pool
difficulty = 800_000
# interval of pulling block template
pull_block_template_interval = 5000
# interval of save log
log_interval = 120
# rpc timeout
rpc_timeout = 2000
# rpc retry times
rpc_retry_times = 5
# alert url, if set, will send alert message to this url
alert_url = ""

# block assembler configuration
[block_assembler]
code_hash = "0x"
args = "0x"
hash_type = "type"
message = "0x"

# influxdb configuration, if set, will write share and block to influxdb
[influx]
url = "http://127.0.0.1:4620"
org = ""
token = ""
share_bucket = "share-bucket"
block_bucket = "block-bucket"
log_bucket = "log-bucket"

# log configuration
[log_config]
# log filter
filter = "info"
# rolling file configuration, index 0 is for log file directory, index 1 is for log file name prefix
rolling_file = ["./logs", "fi5pool"]

# mailer configuration, if set, will send alert message to this mailer
[mailer]
username = ""
password = ""
relay = ""
```

## Deploy
Inspect deploy [readme](./deploy/README.md)

## License

Fi5Pool is licensed under the [Apache 2.0 License](LICENSE).
