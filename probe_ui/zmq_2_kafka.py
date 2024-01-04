#!/usr/bin/env python

from confluent_kafka import Producer
from confluent_kafka.admin import AdminClient, NewTopic
import zmq
import json
import argparse

# Callback function to check the status of the sent messages
def delivery_report(err, msg):
    if err is not None:
        print(f'Message delivery failed: {err}')
    else:
        print(f'Message delivered to {msg.topic()} [{msg.partition()}]')

def main():
    context = zmq.Context()
    socket = context.socket(zmq.SUB)
    socket.connect(f"{args.zmq}")
    socket.setsockopt_string(zmq.SUBSCRIBE, "")
    admin_client = AdminClient({'bootstrap.servers': kafka_conf['bootstrap.servers']})

    while True:
        message = socket.recv_string()
        #print(f"Received Message: {message}")

        data = json.loads(message)
        if 'dst' not in data:
            print(f"No Destination in data! {message}")
            time.sleep(0.10)
            continue

        kafka_topic = 'test'
        #kafka_topic = data['dst']
        print(f"Service {kafka_topic} sending message")
        kafka_topic = kafka_topic.replace(':', '_')
        kafka_topic = kafka_topic.replace('.', '_')
        topic_metadata = admin_client.list_topics(timeout=10)
        print(f"Topics on server {topic_metadata}")
        if kafka_topic not in topic_metadata.topics:
            print(f"Topic {kafka_topic} does not exist. Creating it.")
            topic_list = [NewTopic(kafka_topic, num_partitions=1, replication_factor=1)]
            admin_client.create_topics(topic_list)

        print(f"Forwarding message for topic {kafka_topic} to kafka server {kafka_conf}")

        # Asynchronously produce a message to the Kafka topic
        # the delivery report callback is triggered from poll() or flush()
        producer.produce(kafka_topic, message.encode('utf-8'), callback=delivery_report)

        # Trigger any available delivery report callbacks from previous produce() calls
        producer.poll(0)

if __name__ == "__main__":

    parser = argparse.ArgumentParser()
    parser.add_argument('--kafka', help='Kafka server to connect to', default='127.0.0.1:9092', type=str)
    parser.add_argument('--zmq', help='ZMQ server to connect to', default='tcp://127.0.0.1:1000', type=str)
    args = parser.parse_args()

    # Kafka configuration settings
    kafka_conf = {
        'bootstrap.servers': f"{args.kafka}",  # Change this to your Kafka server address
        'client.id': 'zmq-to-kafka'
    }

    # Initialize Kafka Producer
    producer = Producer(kafka_conf)

    main()

