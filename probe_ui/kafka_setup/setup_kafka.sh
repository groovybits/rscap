#!/bin/bash

set -e

sudo yum install java-11-openjdk.x86_64 -y

# Kafka Installation
sudo adduser kafka
sudo passwd kafka  # Set a strong password for the kafka user
wget https://downloads.apache.org/kafka/3.6.0/kafka_2.13-3.6.0.tgz
tar -xzf kafka_2.13-3.6.0.tgz
sudo mv kafka_2.13-3.6.0 /opt/kafka

# Prometheus Installation
wget https://github.com/prometheus/prometheus/releases/download/v2.47.1/prometheus-2.47.1.linux-amd64.tar.gz
tar -xzf prometheus-2.47.1.linux-amd64.tar.gz
sudo mv prometheus-2.47.1.linux-amd64 /opt/prometheus

# Grafana Installation
sudo tee /etc/yum.repos.d/grafana.repo <<EOF
[grafana]
name=Grafana
baseurl=https://packages.grafana.com/oss/rpm
repo_gpgcheck=1
enabled=1
gpgcheck=1
gpgkey=https://packages.grafana.com/gpg.key
sslverify=1
sslcacert=/etc/pki/tls/certs/ca-bundle.crt
EOF
sudo yum install grafana -y

# Start Kafka server
sudo /opt/kafka/bin/kafka-server-start.sh /opt/kafka/config/server.properties &

# Start Prometheus server
sudo /opt/prometheus/prometheus --config.file=/opt/prometheus/prometheus.yml &

# Start Grafana server
sudo systemctl start grafana-server
