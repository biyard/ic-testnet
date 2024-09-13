REPLICA_VERSION = 0.9.0
BASE_DIR = $(shell pwd)/tmp

setup:
	yay -S gnu-netcat

ic/target/release/replica:
	cd ic && cargo build --bin replica --release

run: build-deps clean ic/target/release/replica tmp
	mkdir -p logs
	ic/target/release/replica --replica-version $(REPLICA_VERSION) --config-file $(BASE_DIR)/ic-100.json5 > logs/node-100.log &
	ic/target/release/replica --replica-version $(REPLICA_VERSION) --config-file $(BASE_DIR)/ic-101.json5 > logs/node-101.log &
	ic/target/release/replica --replica-version $(REPLICA_VERSION) --config-file $(BASE_DIR)/ic-102.json5 > logs/node-102.log &
	ic/target/release/replica --replica-version $(REPLICA_VERSION) --config-file $(BASE_DIR)/ic-103.json5 > logs/node-103.log &

run-nodes:
	docker exec node1 ic/target/release/replica --replica-version 0.9.0 --config-file tmp/state-100/ic.json5

tmp:
	BASE_DIR=$(BASE_DIR) cargo run --bin ic-testnet --
	cp -rf $(BASE_DIR)/state $(BASE_DIR)/state-100
	cp -rf $(BASE_DIR)/state $(BASE_DIR)/state-101
	cp -rf $(BASE_DIR)/state $(BASE_DIR)/state-102
	cp -rf $(BASE_DIR)/state $(BASE_DIR)/state-103

clean:
	rm -rf tmp logs

build-deps:
	cd ic && cargo build --bin replica --release

compose: ic/target/debug/replica tmp
	PWD=$(PWD) docker-compose up -d

kill:
	killall replica
