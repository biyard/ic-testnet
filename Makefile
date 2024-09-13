REPLICA_VERSION = 0.9.0

setup:
	yay -S gnu-netcat

ic/target/release/replica:
	cd ic && cargo build --bin replica --release

run: build-deps clean ic/target/release/replica tmp
	mkdir -p logs
	ic/target/release/replica --replica-version $(REPLICA_VERSION) --config-file tmp/state-100/ic-100.json5 > logs/node-100.log &
	ic/target/release/replica --replica-version $(REPLICA_VERSION) --config-file tmp/state-101/ic-101.json5 > logs/node-101.log &
	# ic/target/release/replica --replica-version $(REPLICA_VERSION) --config-file tmp/state-102/ic.json5 > logs/node-102.log &
	# ic/target/release/replica --replica-version $(REPLICA_VERSION) --config-file tmp/state-103/ic.json5 > logs/node-103.log &

run-nodes:
	docker exec node1 ic/target/release/replica --replica-version 0.9.0 --config-file tmp/state-100/ic.json5

tmp:
	cargo run --bin ic-testnet --
	cp -rf tmp/state-100/* tmp/state-101/

clean:
	rm -rf tmp logs

build-deps:
	cd ic && cargo build --bin replica --release

compose: ic/target/debug/replica tmp
	PWD=$(PWD) docker-compose up -d
