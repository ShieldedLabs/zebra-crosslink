#!/bin/sh

if [[ -z "$1" ]]; then
    RED='\033[0;31m'
    NC='\033[0m'
    echo -e "${RED}Error!${NC} You must give the path to a zebra cache as the first argument."
    echo "    e.g.  docker/run_all_tests_once.sh /home/sam/.cache/zebra/"
    echo ""
    echo "Zebra cache is 250 GiB and will take days to sync organically. I have created a"
    echo "zip file that you can download instead. http://70.34.198.4/zebra_cache.zip"
    exit 1
fi

ZC=$(realpath $1)

cd $(dirname $0)/..

export ZEBRA_CACHED_STATE_DIR=$ZC
export TEST_LWD_UPDATE_SYNC=
export RUN_ALL_TESTS=1
export TEST_LWD_GRPC=
export SHORT_SHA=
export LOG_COLOR=false
export TEST_ZEBRA_EMPTY_SYNC=
export RUST_VERSION=1.82.0
export FULL_SYNC_MAINNET_TIMEOUT_MINUTES=
export TEST_LWD_RPC_CALL=
export RUST_BACKTRACE=1
export RUST_LOG=info
export FULL_SYNC_TESTNET_TIMEOUT_MINUTES=

export PATH=$(pwd)/target/debug:$PATH

CARGO_ARGS="--locked --features \"default-release-binaries lightwalletd-grpc-tests zebra-checkpoints\" --workspace -- --nocapture --include-ignored"

set -e
set -o pipefail

if [[ "$2" ]]; then
    eval exec cargo test $2 $CARGO_ARGS
fi

eval cargo test $CARGO_ARGS \
--skip config_tests \
--skip zebrad_update_sync \
--skip lightwalletd_test_suite \
--skip get_block_template \
--skip submit_block \
--skip fully_synced_rpc_z_getsubtreesbyindex_snapshot_test \
--skip scan_start_where_left \
--skip scan_task_commands \
--skip generate_checkpoints_testnet # This test is disabled because we currently do not want to
                                    # require having the testnet chain stored locally.

eval cargo test config_tests $CARGO_ARGS
eval cargo test zebrad_update_sync $CARGO_ARGS
eval cargo test lightwalletd_test_suite $CARGO_ARGS
eval cargo test submit_block $CARGO_ARGS
eval cargo test fully_synced_rpc_z_getsubtreesbyindex_snapshot_test $CARGO_ARGS
eval cargo test scan_start_where_left $CARGO_ARGS
eval cargo test scan_task_commands $CARGO_ARGS
