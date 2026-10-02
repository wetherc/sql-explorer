#!/bin/sh
# Runs the live tests against the servers of compose.yaml.
#
# A variable that is already set wins, so the tests can also run against
# another server. The password of SQL Server contains a number sign, which the
# URL gives as %23.

set -e

cd "$(dirname "$0")/.."

: "${SQLX_LIVE_PG:=postgres://postgres:LivePg16pass@127.0.0.1:15416}"
: "${SQLX_LIVE_MYSQL:=mysql://root:LiveMysql84pass@127.0.0.1:13384}"
: "${SQLX_LIVE_MARIADB:=mysql://root:LiveMaria11pass@127.0.0.1:13311}"
: "${SQLX_LIVE_MSSQL:=mssql://sa:Live%23Mssql2022Pass@127.0.0.1:11433}"
export SQLX_LIVE_PG SQLX_LIVE_MYSQL SQLX_LIVE_MARIADB SQLX_LIVE_MSSQL

# The filter `live_` selects the live tests. An argument, such as the name
# of one test, takes the place of that filter. `--include-ignored` runs the
# tests that have `#[ignore]`.
if [ "$#" -eq 0 ]; then
    set -- live_
fi
exec cargo test -- --include-ignored "$@"
