REPLICA_VERSION ?= 0.9.0
BASE_DIR ?= $(shell pwd)/tmp
ENV ?= default
NODES ?= "127.0.1.1 127.0.2.1 127.0.3.1 127.0.4.1"

setup:
	yay -S gnu-netcat

ic/target/release/replica:
	cd ic && cargo build --bin replica --release

ic/target/debug/replica:
	cd ic && cargo build --bin replica

run: clean ic/target/debug/replica
	NODES=$(NODES) BASE_DIR=$(BASE_DIR) cargo run --
	cp -rf $(BASE_DIR)/state $(BASE_DIR)/state-100
	cp -rf $(BASE_DIR)/state $(BASE_DIR)/state-101
	cp -rf $(BASE_DIR)/state $(BASE_DIR)/state-102
	cp -rf $(BASE_DIR)/state $(BASE_DIR)/state-103

	mkdir -p logs
	ic/target/debug/replica --replica-version $(REPLICA_VERSION) --config-file $(BASE_DIR)/ic-100.json5 > logs/node-100.log &
	ic/target/debug/replica --replica-version $(REPLICA_VERSION) --config-file $(BASE_DIR)/ic-101.json5 > logs/node-101.log &
	ic/target/debug/replica --replica-version $(REPLICA_VERSION) --config-file $(BASE_DIR)/ic-102.json5 > logs/node-102.log &
	ic/target/debug/replica --replica-version $(REPLICA_VERSION) --config-file $(BASE_DIR)/ic-103.json5 > logs/node-103.log &


run-docker: docker-clean build-deps ic/target/debug/replica tmp start-docker

start-docker:
	PWD=$(PWD) docker-compose up -d

start:
	mkdir -p logs
	ic/target/debug/replica --replica-version $(REPLICA_VERSION) --config-file $(BASE_DIR)/ic-100.json5 > logs/node-100.log &
	ic/target/debug/replica --replica-version $(REPLICA_VERSION) --config-file $(BASE_DIR)/ic-101.json5 > logs/node-101.log &
	ic/target/debug/replica --replica-version $(REPLICA_VERSION) --config-file $(BASE_DIR)/ic-102.json5 > logs/node-102.log &
	ic/target/debug/replica --replica-version $(REPLICA_VERSION) --config-file $(BASE_DIR)/ic-103.json5 > logs/node-103.log &


start.%:
	ic/target/debug/replica --replica-version $(REPLICA_VERSION) --config-file $(BASE_DIR)/ic-$*.json5 > logs/node-$*.log &

run-nodes:
	docker exec node1 ic/target/debug/replica --replica-version 0.9.0 --config-file tmp/state-100/ic.json5

tmp:
	mkdir -p tmp
	BASE_DIR=$(BASE_DIR) cargo run --
	cp -rf tmp/state tmp/state-100
	cp -rf tmp/state tmp/state-101
	cp -rf tmp/state tmp/state-102
	cp -rf tmp/state tmp/state-103

clean:
	rm -rf tmp logs

docker-clean:
	sudo rm -rf tmp logs
	docker-compose down

build-deps:
	cd ic && cargo build --bin replica

compose: ic/target/debug/replica tmp
	PWD=$(PWD) docker-compose up -d

kill:
	killall replica
