// server.js
const { Kafka } = require('kafkajs');
const express = require('express');
const { Server } = require('ws');

const app = express();
const server = require('http').createServer(app);
const wss = new Server({ server });

let debug = false;

app.use(express.static('public'));  // This line serves static files from the 'public' directory

const kafka = new Kafka({
  clientId: 'test',
  brokers: ['sun:9092']
});

const consumer = kafka.consumer({ groupId: 'test' });

wss.on('connection', ws => {
  ws.on('message', message => {
    console.log('received: %s', message);
  });
});

function isJson(str) {
  try {
      JSON.parse(str);
  } catch (e) {
      return false;
  }
  return true;
}

function checkRequiredFields(json) {
  const requiredStatsFields = ['mbps', 'ccerrors', 'packetcount'];
  const requiredPidsFields = ['packetcount'];
  const requiredServicesFields = ['streams'];

  if (!json.stats || !json.pids || !json.services) return false;
  for (const field of requiredStatsFields) {
      if (!json.stats.hasOwnProperty(field)) return false;
  }
  for (const pid of json.pids) {
      for (const field of requiredPidsFields) {
          if (!pid.hasOwnProperty(field)) return false;
      }
  }
  for (const service of json.services) {
      for (const field of requiredServicesFields) {
          if (!service.hasOwnProperty(field)) return false;
      }
  }

  return true;
}

async function fetchData() {
  await consumer.connect();
  await consumer.subscribe({ topic: 'test', fromBeginning: false });

  await consumer.run({
    eachMessage: async ({ message }) => {
      const messageStr = message.value.toString();  // Convert Buffer to string

      // remove ^posting html json:$ string from top of message if exists
      if (messageStr.indexOf('^posting html json:$') === 0) {
        messageStr = messageStr.substring(20);
      }

      // check if message is empty or just empty spaces or just new lines
      if (messageStr === '' || messageStr.trim() === '' || messageStr === '\n') {
        console.error("Message is empty", {messageStr});
        return;
      }

      if (!isJson(messageStr)) {
        console.error('Received non-JSON message:', messageStr);
        return;
      }

      let messageJson;
      try {
        messageJson = JSON.parse(messageStr);  // Try to parse the string as JSON

        // check dst field "dst":"224.0.0.200:10000" to confirm we are coming from the right source, confirm it exists and be safe about checking it
        if (messageJson.dst !== undefined || messageJson.dst !== null || messageJson.dst !== '') {
          // check dst field for our target ip address and port
          let targetIp = '224.0.0.200';
          let targetPort = '10000';
          if (messageJson.dst.indexOf(targetIp) === 0 && messageJson.dst.indexOf(targetPort) > -1) {
            // if dst field is present and contains our target ip address and port, continue
            //console.log(`Message:`, messageJson);
          } else {
            // if dst field is present but does not contain our target ip address and port, return
            return;
          }
        } else {
          // if dst field is not present, return
          return;
        }

      } catch (e) {
        console.error(`Failed to parse message:`, e, `\nMessage: '`, messageStr, `'`);
        return;
      }

      // Check for required fields
      if (!checkRequiredFields(messageJson)) {
        console.error('Message missing required fields:', messageJson);
        return;
      }

      const messageToSend = JSON.stringify(messageJson);  // Stringify the JSON
      wss.clients.forEach(client => {
        client.send(messageToSend);  // Send the properly formatted JSON string
      });
    }
  });
}

fetchData();

server.listen(3001, '127.0.0.1', () => {
  console.log('Server is listening on http://127.0.0.1:3001');
});
