#!/bin/bash

#       ./start_influx.sh http://127.0.0.1:8086

INFLUX_TOKEN=$(<admin-token)

wget https://dl.influxdata.com/influxdb/releases/influxdb2-client-2.7.5-linux-amd64.tar.gz

tar xvzf ./influxdb2-client-2.7.5-linux-amd64.tar.gz

chmod +x influx

./influx bucket create --host "$1" --token $INFLUX_TOKEN --name share-bucket --org fi5 --retention 60h
./influx bucket create --host "$1" --token $INFLUX_TOKEN --name block-bucket --org fi5 --retention 0
./influx bucket create --host "$1" --token $INFLUX_TOKEN --name log-bucket --org fi5 --retention 60d
