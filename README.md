# Tycho L2

A set of tools for Tycho Trustless Bridge (based on [TON Trustless Bridge](https://github.com/tonred/ton-trustless-bridge/tree/tycho) by TONRED).

## Contents

- [Proof API for L2](#proof-api-l2)
- [Proof API for TON](#proof-api-ton)
- [Sync Service](#sync-service)

---

### Proof API L2

A standalone light node for Tycho-based networks. It syncs all blocks in real-time and provides an API for building proof chains.

```bash
# Install.
cargo install --path ./proof-api-l2 --locked

# Generate and edit the default config.
proof-api-l2 run --init-config config.json

# Download the latest global config (Tycho).
wget -O global-config.json https://testnet.tychoprotocol.com/global-config.json

# Start the node.
proof-api-l2 run \
  --config config.json \
  --global-config global-config.json \
  --keys keys.json
```

<details><summary><b>Example config</b></summary>
<p>

```json
{
  "public_ip": null,
  "local_ip": "0.0.0.0",
  "port": 30000,
  "storage": {
    "root_dir": "./db",
    "rocksdb_enable_metrics": true,
    "rocksdb_lru_capacity": "22.3 GB",
    "cells_cache_size": "4.3 GB",
    "archive_chunk_size": "1024.0 KB",
    "split_block_tasks": 100,
    "archives_gc": {
      "persistent_state_offset": "5m"
    },
    "states_gc": {
      "random_offset": true,
      "interval": "1m"
    },
    "blocks_gc": {
      "type": "BeforeSafeDistance",
      "safe_distance": 1000,
      "min_interval": "1m",
      "enable_for_sync": true,
      "max_blocks_per_batch": 100000
    },
    "blocks_cache": {
      "ttl": "5m",
      "size": "500.0 MB"
    }
  },
  "metrics": {
    "listen_addr": "127.0.0.1:10000"
  },
  "api": {
      "listen_addr": "127.0.0.1:8080",
      "public_url": null
  },
  "proof_storage": {
      "rocksdb_lru_capacity": "3.7 GiB",
      "rocksdb_enable_metrics": false,
      "min_proof_ttl": "14days",
      "compaction_interval": "10m"
  }
}
```
</p>
</details>

#### API Schema.

Example:
```bash
export addr="-1:3333333333333333333333333333333333333333333333333333333333333333"
export tx_lt="609171000001"
curl "http://127.0.0.1:8080/v1/proof_chain/${addr}/${tx_lt}"
```

Output:
```json
{
  "proofChain": "te6ccgECIgEABRIACUYD7HYzZ1cEEE6TR87CnIRVV6JUGTmYGei95521rVzvpaEAGwEiSNpgaqL0mZ/aAh08XG7ZJrQjpYo8sPg8OwKOXlryRraLZ+tE1xMCAgLMBgMCAUgFBACBVfpXaVStyx3jm6ICjxD0gSPzIqc+5U3SvszrX96/RiEYzqCjKWCwV1C7DyW0l7WidSnOXzIH2kHUeOCA+lg0EEgAgVA0oYlnGVATsLkWrBtzLyQ2Wck5kKjMurotRWVJ5rFUVqguzAnFNkbHSy4JQ3WSI74CQSP3QNKcG5dtCQ+YKxAYAgEgDgcCASALCAIBIAoJAIES3+Lo6Bmd7BKe5IZ3OmMrHRkJIlsmsdkIjOq5yNd8bbd2NaLaxW+j9nx6i3A73/NemqosYp0vpCix7u3472fDoACBItjd+n/vifX4mB70rKowvUGdiMuzBn3hXY7OwG0lyrSc7Ye1FuSq5q1U4aJq/nkeIlcuU2U3a1CRfTSUefyqASACASANDACBB40wFv2k6wzT6Mpz5ou2FwpLHbzOOz/IhROCgae2MwdZk3vkwptWAhhs6MZge/qtViQASX49Yyej+ePnwag7QmAAgTI38C0RAZCgn5g1MlHaB5QMY+/4no2k4VhcrMF1HvpExdhKFIwvvnTnFUB/n/K91VEJFcr+3I6IjjM4T3P3YoPgAgEgEA8AgVla4D9AoSmpX3qfM1CBSc8qFAf8BG41fpfz7LSuDLNE2NEL/QIgdwDLLZngknfTQ1GKXEnD3JMBxbAYSe8zrcDoAgEgEhEAgRamwvIC76LeCmiszvEyQznFbO6kBj9Mmt6MAQxdsiEWOZRbmkXwReiHr5NxJamWZrglUX6iL698Khwyx7hpL8IgAIEAJ4KkpIGs7MpWLd+RnxWgiuZGBiIhU0iLPQKh7Dqtlhk6SpoCRLZwaDNF2Wbnb0aB96UBUnpZy/AfivM0jNfAICQQEe9Vu///6I8gHx4UJIlKM/b8TSZFFMngEHAzzQg1y9K+bn2K8Oo4x0yF0wSub/H5ZWhoyedRm6soO+rtymuULxXD+LMQNWUybAxQQgR7j8jyIcAdHBYVKEgBAUdZzSAeM5nyzl6l2wKsX5HvkjvLfh8rOICjun15ruE2AAMhAYIXIgNAQBkYKEgBARrqkr/L62NHJmeadvi54aJSUvPo/C0+fNw/j10IgKSsAAMil7+zMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMwKZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmZmdAAAAAjdVrysEEbGihIAQF4sgwDm/A/PygF6X2saciuJp3mf3qf5hFGDLT60B9R+AAAKEgBAUklMHQ0qXomF/veUEBZDdcJ0Ozdr2RrcyjnH/Uq6Ma/AAIAAQIAAwAgKEgBAStnMWLHYGFUFZSJVDEQscal87WgNsAbhz265b9wclA+ABkoSAEBve8hrPYYGaAXarcjytHbil31ii5KYyUTh1RBzV/YizwAAQGkm8epiAAAAAAAAQAIOqoAAAAAAP////8AAAAAAAAAAGfrzIgA2AAAAI3Va8rAAAAAjdVrysOhxsNrAFsldAAIOpoACBLMxAAAACAAAAAADAMWLiEAmAAAAI3VXIiDAAg6qZHwLdCzgyhrg9kRW3kizwtHSyH/JQMCltcq8amKcxHT/uCf1sMFy4DNIZQdF8MXqEVjb76Xj6ZDKOr/K0KE4ig="
}
```

<details><summary><b>Swagger</b></summary>
<p>

```yaml
---
openapi: 3.1.0
info:
  title: ''
  description: proof-api-l2
  version: 0.1.0 (build unknown)
servers:
- url: http://127.0.0.1:8080
  description: local
paths:
  "/":
    get:
      description: Get the API version
      responses:
        '200':
          description: ''
          content:
            application/json:
              schema:
                "$ref": "#/components/schemas/ApiInfoResponse"
  "/v1/proof_chain/{address}/{lt}":
    get:
      tags:
      - proof-api-l2
      description: Build proof chain
      responses:
        '200':
          description: ''
          content:
            application/json:
              schema:
                "$ref": "#/components/schemas/ProofChainResponse"
        '404':
          description: no content
        '500':
          description: ''
          content:
            application/json:
              schema:
                "$ref": "#/components/schemas/ErrorResponse"
  "/api.json":
    get:
      tags:
      - swagger
components:
  schemas:
    Address:
      description: StdAddr in any format
      examples:
      - 0:3333333333333333333333333333333333333333333333333333333333333333
      type: string
      format: 0:[0-9a-fA-F]{64}
    ApiInfoResponse:
      description: API version and build information.
      type: object
      required:
      - build
      - version
      properties:
        build:
          type: string
        version:
          type: string
    ErrorResponse:
      description: General error response.
      oneOf:
      - type: object
        required:
        - error
        - message
        properties:
          error:
            type: string
            enum:
            - internal
          message:
            type: string
      - type: object
        required:
        - error
        - message
        properties:
          error:
            type: string
            enum:
            - notFound
          message:
            type: string
    ProofChainResponse:
      description: Block proof chain for an existing transaction.
      type: object
      required:
      - proofChain
      properties:
        proofChain:
          description: Base64 encoded BOC with the proof chain.
          type: string
```

</p>
</details>

---

### Proof API TON

A wrapper around LightClient for TON-based networks. It provides an API for building proof chains.

```bash
# Install.
cargo install --path ./proof-api-ton --locked

# Generate and edit the default config.
proof-api-ton run --init-config config.json

# Download the latest global config (TON).
wget -O global-config.json https://ton.org/global.config.json

# Start the API.
proof-api-ton run \
  --config config.json \
  --global-config global-config.json
```

<details><summary><b>Example config</b></summary>
<p>

```json
{
  "api": {
    "listen_addr": "127.0.0.1:8080",
    "public_url": null,
    "rate_limit": 400,
    "whitelist": []
  },
  "logger_config": {
    "outputs": [
      {
        "type": "Stderr"
      }
    ]
  }
}
```

</p>
</details>

#### API Schema.

Example:
```bash
export addr="EQCxE6mUtQJKFnGfaROTKOt1lZbDiiX1kCixRv7Nw2Id_sDs"
export tx_lt="55597705000001"
export tx_hash="5cd501601f6fc3ebfd998d752ffd8efbb237c20511989f576405383f1766cc9f"
curl "http://127.0.0.1:8080/v1/proof_chain/${addr}/${tx_lt}/${tx_hash}"
```

Output:
```json
{
  "proofChain": "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"
}
```

<details><summary><b>Swagger</b></summary>
<p>

```yaml
---
openapi: 3.1.0
info:
  title: ''
  description: proof-api-ton
  version: 0.1.0 (build unknown)
servers:
- url: http://127.0.0.1:8080
  description: local
paths:
  "/":
    get:
      description: Get the API version
      responses:
        '200':
          description: ''
          content:
            application/json:
              schema:
                "$ref": "#/components/schemas/ApiInfoResponse"
  "/v1/proof_chain/{address}/{lt}/{hash}":
    get:
      tags:
      - proof-api-ton
      description: Build proof chain
      responses:
        '200':
          description: ''
          content:
            application/json:
              schema:
                "$ref": "#/components/schemas/ProofChainResponse"
        '404':
          description: no content
        '500':
          description: ''
          content:
            application/json:
              schema:
                "$ref": "#/components/schemas/ErrorResponse"
  "/api.json":
    get:
      tags:
      - swagger
components:
  schemas:
    Address:
      description: StdAddr in any format
      examples:
      - 0:3333333333333333333333333333333333333333333333333333333333333333
      type: string
      format: 0:[0-9a-fA-F]{64}
    ApiInfoResponse:
      description: API version and build information.
      type: object
      required:
      - build
      - version
      properties:
        build:
          type: string
        version:
          type: string
    ErrorResponse:
      description: General error response.
      oneOf:
      - type: object
        required:
        - error
        - message
        properties:
          error:
            type: string
            enum:
            - internal
          message:
            type: string
      - type: object
        required:
        - error
        - message
        properties:
          error:
            type: string
            enum:
            - notFound
          message:
            type: string
      - type: object
        required:
        - error
        properties:
          error:
            type: string
            enum:
            - limitExceed
    ProofChainResponse:
      description: Block proof chain for an existing transaction.
      type: object
      required:
      - proofChain
      properties:
        proofChain:
          description: Base64 encoded BOC with the proof chain.
          type: string
    Transaction hash:
      description: Transaction hash as hex
      examples:
      - '3333333333333333333333333333333333333333333333333333333333333333'
      type: string
      format: "[0-9a-fA-F]{64}"
```

</p>
</details>

---

### Sync Service

A service for validator set synchronization between any networks.

```bash
# Install.
cargo install --path ./sync-service --locked

# Generate wallet.
export WALLET=$(sync-service account)
# Example:
# {
#  "secret": "dcc10f69ff2d8b68851f27e9ac036c07305236dc661e6859786b604161c12247",
#  "public": "2b4aca5bdfdb9e549a3313216e989051ba8a6c62034b957bf51f7257f89a5c35",
#  "address": "0:fb08b6d52c6c687bdc1b4e4e3b5bc62d7242fc5828c074fd37daece4b49b4aca"
#}

# Use wallet address and secret to build the config (see example).
export wallet_address=$(echo "$WALLET" | jq -r ".address")
export wallet_secret=$(echo "$WALLET" | jq -r ".secret")

# Topup the wallet in every network.

# Download required TON global configs
wget -O mainnet-global-config.json https://ton.org/mainnet-global.config.json
wget -O testnet-global-config.json https://ton.org/testnet-global.config.json

# Start the service.
sync-service run --config config.json
```

#### Example config

```json
{
  "workers": [
    {
      "left": {
        "type": "Ton",
        "name": "ton_testnet",
        "global_config": "./testnet-global-config.json",
        "uploader": {
          "bridge_address": "EQDdxyy8DtX1uH8aDJaaGaC2Wt6W19nQapjaYuJz3Kn8ZbQM",
          "lib_store_value": "30000000",
          "store_vset_value": "100000000",
          "wallet_address": "0:fb08b6d52c6c687bdc1b4e4e3b5bc62d7242fc5828c074fd37daece4b49b4aca",
          "wallet_secret": "dcc10f69ff2d8b68851f27e9ac036c07305236dc661e6859786b604161c12247"
        }
      },
      "right": {
        "type": "Tycho",
        "name": "tycho_devnet1",
        "rpc": "https://rpc-devnet1.tychoprotocol.com",
        "uploader": {
          "bridge_address": "EQAondW7AWhz3Y3qEN8AvfCzotmGcJ6LInMe6An1CZgLpgOR",
          "lib_store_value": "10000000000",
          "store_vset_value": "5000000000",
          "wallet_address": "0:fb08b6d52c6c687bdc1b4e4e3b5bc62d7242fc5828c074fd37daece4b49b4aca",
          "wallet_secret": "dcc10f69ff2d8b68851f27e9ac036c07305236dc661e6859786b604161c12247"
        }
      }
    }
  ]
}
```

## Contributing

We welcome contributions to the project! If you notice any issues or errors,
feel free to open an issue or submit a pull request.

## License

Licensed under either of

* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE)
  or <https://www.apache.org/licenses/LICENSE-2.0>)
* MIT license ([LICENSE-MIT](LICENSE-MIT)
  or <https://opensource.org/licenses/MIT>)

at your option.
