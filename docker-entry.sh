#!/bin/bash
echo $1
pwd
ic/target/debug/replica --replica-version 0.9.0 --config-file tmp/ic-$1.json5
