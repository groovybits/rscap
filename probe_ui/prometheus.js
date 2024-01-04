const { Kafka } = require('kafkajs');
const express = require('express');
const promClient = require('prom-client');

const app = express();
const kafka = new Kafka({ clientId: 'vma', brokers: ['sun:9092'] });
const consumer = kafka.consumer({ groupId: 'prometheus' });

// Create Prometheus Metrics
const createGauge = (name, help, labelNames = []) => new promClient.Gauge({ name, help, labelNames });

/*
# HELP kafka_pid_packet_count Packet count per PID
# TYPE kafka_pid_packet_count gauge

# HELP kafka_pid_cc_errors CC errors per PID
# TYPE kafka_pid_cc_errors gauge

# HELP kafka_pid_mbps Mbps per PID
# TYPE kafka_pid_mbps gauge

# HELP kafka_stats_mbps Mbps from stats
# TYPE kafka_stats_mbps gauge

# HELP kafka_la1 Load Average 1 minute
# TYPE kafka_la1 gauge

# HELP kafka_iat1_min Minimum IAT1
# TYPE kafka_iat1_min gauge

# HELP kafka_iat1_max Maximum IAT1
# TYPE kafka_iat1_max gauge

# HELP kafka_iat1_avg Average IAT1
# TYPE kafka_iat1_avg gauge

# HELP kafka_tr101290 TR101290 Status
# TYPE kafka_tr101290 gauge

# HELP kafka_service_stream_count Number of streams per service
# TYPE kafka_service_stream_count gauge
*/
/*
    {"host":"vma-dev02","timestamp":"2023-11-17 05:55:54.000","type":"UDP","src":"192.168.50.13:62628","dst":"224.0.0.200:10000",
            "la1":0.10000000000000001,
            "la5":0.050000000000000003,
            "la15":0.050000000000000003,
            "stats":{"mbps":9.9910720000000008,
                    "ccerrors":0,
                    "packetcount":152369,
                    "nic":"eth0",
                    "pcap_ifdrop":0,
                    "pcap_psdrop":0,
                    "iat1_min":0,
                    "iat1_max":0,
                    "iat1_avg":-1,
                    "warning_indicators":"---T"},
            "services":[{"program":1,
                        "pmtpid":"0x0030",
                        "pcrpid":"0x0031",
                        "escount":3,
            "streams":[{"pid":"0x0031",
                        "type":"0x1b",
                        "desc":"H.264 Video"},
                        {"pid":"0x0032",
                        "type":"0x04",
                        "desc":"ISO\/IEC 13818-3 Audio"},
                        {"pid":"0x0033",
                        "type":"0x86",
                        "desc":"User Private"}]}],
            "tr101290":{"p1":{"tssyncloss":"BAD","syncbyte":"OK ","pat":"OK ","pat2":"OK ","cc":"OK ","pmt":"OK ","pmt2":"OK ","pid":"BAD 0x0033 "},
                        "p2":{"transport":"OK ","crc":"OK  ","pcr":"OK  ","pcrrep":"OK  ","cat":"OK "},
                        "p3":{}},
            "pids":[{"pid":"0x0000",
                        "packetcount":241,
                        "ccerrors":0,
                        "mbps":0.01504},
                    {"pid":"0x0030","packetcount":241,"ccerrors":0,"mbps":0.01504},
                    {"pid":"0x0031","packetcount":135093,"ccerrors":0,"mbps":9.2601279999999999},
                    {"pid":"0x0032","packetcount":4058,"ccerrors":0,"mbps":0.266208},
                    {"pid":"0x1fff","packetcount":12736,"ccerrors":0,"mbps":0.452704}]}
*/
const metrics = {
    packetCount: createGauge('kafka_pid_packet_count', 'Packet count per PID', ['dst', 'pid']),
    ccErrors: createGauge('kafka_pid_cc_errors', 'CC errors per PID', ['dst', 'pid']),
    mbps: createGauge('kafka_pid_mbps', 'Mbps per PID', ['dst', 'pid']),
    statsMpbs: createGauge('kafka_stats_mbps', 'Mbps from stats', ['dst']),
    la1: createGauge('kafka_la1', 'Load Average 1 minute', ['dst']),
    la5: createGauge('kafka_la5', 'Load Average 5 minute', ['dst']),
    la15: createGauge('kafka_la15', 'Load Average 15 minute', ['dst']),
    iat1Min: createGauge('kafka_iat1_min', 'Minimum IAT1', ['dst']),
    iat1Max: createGauge('kafka_iat1_max', 'Maximum IAT1', ['dst']),
    iat1Avg: createGauge('kafka_iat1_avg', 'Average IAT1', ['dst']),
    tr101290: createGauge('kafka_tr101290', 'TR101290 Status', ['dst', 'p', 'metric']),
    streams: createGauge('kafka_stream', 'Stream', ['dst', 'pid', 'type', 'desc']),
    services: createGauge('kafka_service', 'Service', ['dst', 'program', 'pmtpid', 'pcrpid', 'escount']),
    totalPacketCount: createGauge('kafka_total_packet_count', 'Total Packet Count', ['dst']),
    totalCCerrors: createGauge('kafka_total_cc_errors', 'Total CC Errors', ['dst']),
    pcapIfdrop: createGauge('kafka_pcap_ifdrop', 'PCAP IF Drop', ['dst']),
    pcapPsdrop: createGauge('kafka_pcap_psdrop', 'PCAP PS Drop', ['dst']),
    // ... Additional metrics definitions ...
};

// Utility function to convert TR101290 status to numeric
function convertToNumeric(status) {
    // Check if status is undefined or null
    if (!status) {
        console.error('Status is undefined or null');
        return 1; // Default to 1 (error state) if status is undefined or null
    }
    //console.log('status:', status);
    return status.substring(0, 2) === 'OK' ? 0 : 1;
}

const serviceStreamCount = createGauge('kafka_service_stream_count', 'Number of streams per service', ['dst', 'service_index']);

// Kafka Consumer and Metrics Updater
const run = async () => {
    await consumer.connect();
    await consumer.subscribe({ topic: 'test', fromBeginning: false });

    await consumer.run({
        eachMessage: async ({ topic, partition, message }) => {
            try {
                let messageStr = message.value.toString();

                // Validate and Clean Message
                if (messageStr.indexOf('^posting html json:$') === 0) {
                    messageStr = messageStr.substring(20);
                }
                if (!messageStr || !messageStr.trim() || !isJson(messageStr)) {
                    console.error('Invalid message:', messageStr);
                    return;
                }

                const data = JSON.parse(messageStr);
                const dst = data.dst.replace(':', '_');

                // Check if pids is defined and is an array
                if (Array.isArray(data.pids)) {
                    // Update PID Metrics
                    data.pids.forEach(pidData => {
                        ['packetCount', 'ccErrors', 'mbps'].forEach(metric => {
                            metrics[metric].labels(dst, pidData.pid).set(pidData[metric.toLowerCase()]);
                        });
                    });
                } else {
                    console.error('Invalid or missing pids:', data.pids);
                }       

                // Update Stats Metrics
                metrics.statsMpbs.labels(dst).set(data.stats.mbps);
                metrics.la1.labels(dst).set(data.la1);
                metrics.la5.labels(dst).set(data.la5);
                metrics.la15.labels(dst).set(data.la15);
                if (data.stats.packetcount) {
                    metrics.totalPacketCount.labels(dst).set(data.stats.packetcount);
                } else {
                    metrics.totalPacketCount.labels(dst).set(0);
                }
                if (data.stats.ccerrors) {
                    metrics.totalCCerrors.labels(dst).set(data.stats.ccerrors);
                } else {
                    metrics.totalCCerrors.labels(dst).set(0);
                }
                if (data.stats.pcap_ifdrop) {
                    metrics.pcapIfdrop.labels(dst).set(data.stats.pcap_ifdrop);
                } else {
                    metrics.pcapIfdrop.labels(dst).set(0);
                }
                if (data.stats.pcap_psdrop) {
                    metrics.pcapPsdrop.labels(dst).set(data.stats.pcap_psdrop);
                } else {
                    metrics.pcapPsdrop.labels(dst).set(0);
                }
                // ... Update other stats metrics ...

                // Update IAT Metrics
                metrics.iat1Min.labels(dst).set(data.stats.iat1_min);
                metrics.iat1Max.labels(dst).set(data.stats.iat1_max);
                metrics.iat1Avg.labels(dst).set(data.stats.iat1_avg);

                // Handle Services Metrics
                // Check if services is defined and is an array
                if (Array.isArray(data.services)) {
                    data.services.forEach((service, index) => {
                        // Update metrics related to services
                        serviceStreamCount.labels(dst, index.toString()).set(service.streams.length);
                        // Additional metrics for services can be handled here
                        /*
                        "services":[{"program":1,
                        "pmtpid":"0x0030",
                        "pcrpid":"0x0031",
                        "escount":3,
                        */
                        // create a metric for each service indexed by service index
                        //     services: createGauge('kafka_service', 'Service', ['dst', 'program', 'pmtpid', 'pcrpid', 'escount']),
                        //metrics.services.labels(dst, index.toString(), service.program, service.pmtpid, service.pcrpid, service.escount).set(1);
                        // convert service.program string from hex as 0xNN string to an integer
                        const program = parseInt(service.program, 16);
                        metrics.services.labels(dst, service.program, service.pmtpid, service.pcrpid, service.escount).set(program);

                        // Handle Stream Metrics
                        // Check if streams is defined and is an array
                        if (Array.isArray(service.streams)) {
                            // Update metrics related to streams
                            service.streams.forEach(stream => {
                                // Additional metrics for streams can be handled here
                            /*
                            "streams":[{"pid":"0x0031",
                                "type":"0x1b",
                                "desc":"H.264 Video"},
                                {"pid":"0x0032",
                                "type":"0x04",
                                "desc":"ISO\/IEC 13818-3 Audio"},
                                {"pid":"0x0033",
                                "type":"0x86",
                                "desc":"User Private"}]}],
                                */
                                // create a metric for each stream by pid using hex value
                                const pid = parseInt(stream.pid, 16);
                                metrics.streams.labels(dst, stream.pid, stream.type, stream.desc).set(pid);
                            });
                        } else {
                            //console.error('Invalid or missing streams:', data.streams);
                            console.error('Invalid or missing streams:', data)
                        }
                    });
                } else {
                    console.error('Invalid or missing services:', data.services);
                }

                // Handle tr101290 Metrics
                if (data.tr101290) {
                    Object.keys(data.tr101290).forEach(p => {
                        const pMetrics = data.tr101290[p];
                        if (!pMetrics) {
                            console.error(`Invalid or missing tr101290 ${p}:`, pMetrics);
                        } else {
                            Object.keys(pMetrics).forEach(metric => {
                                const value = convertToNumeric(pMetrics[metric]);
                                metrics.tr101290.labels(dst, p, metric).set(value);
                            });
                        }
                    });
                }
            } catch (e) {
                console.error(`Failed to process message: ${message.value.toString()}`, e);
            }
        },
    });
};

// Utility Functions
function isJson(str) {
    try {
        JSON.parse(str);
    } catch (e) {
        return false;
    }
    return true;
}

// Metrics Endpoint for Prometheus
app.get('/metrics', async (req, res) => {
    try {
        res.set('Content-Type', promClient.register.contentType);
        const metrics = await promClient.register.metrics();
        res.end(metrics);
    } catch (error) {
        console.error('Failed to retrieve metrics:', error);
        res.status(500).end('Internal server error');
    }
});

app.listen(3002);

run().catch(console.error);