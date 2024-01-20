#!/bin/bash

set -e

sudo yum install java-11-openjdk.x86_64 -y

# Kafka Installation
sudo adduser kafka
sudo passwd kafka  # Set a strong password for the kafka user
wget https://downloads.apache.org/kafka/3.6.0/kafka_2.13-3.6.0.tgz
tar -xzf kafka_2.13-3.6.0.tgz
sudo mv kafka_2.13-3.6.0 /opt/kafka

# Start Kafka server
sudo /opt/kafka/bin/kafka-server-start.sh /opt/kafka/config/server.properties &

