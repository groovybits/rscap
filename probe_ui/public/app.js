// app.js

const ws = new WebSocket('ws://127.0.0.1:3001');

const mbpsChartCtx = document.getElementById('mbpsChart').getContext('2d');
const iatChartCtx = document.getElementById('iatChart').getContext('2d');
const ccErrorsChartCtx = document.getElementById('ccErrorsChart').getContext('2d');
const pidChartCtx = document.getElementById('pidChart').getContext('2d');
const servicesChartCtx = document.getElementById('servicesChart').getContext('2d');

const mbpsChart = createChart(mbpsChartCtx, 'Mbps', 'line');
const iatChart = createChart(iatChartCtx, 'IAT Interval (Packet Counts)', 'bar');
const ccErrorsChart = createChart(ccErrorsChartCtx, 'CC Errors', 'bar');
const pidChart = createChart(pidChartCtx, 'PID Views', 'bar');
const servicesChart = createChart(servicesChartCtx, 'Services', 'bar');

ws.onmessage = event => {
  const data = JSON.parse(event.data);
  updateDashboard(data);
};

function createChart(ctx, label, type) {
  return new Chart(ctx, {
    type: type,
    data: {
      labels: [],
      datasets: [{
        label: label,
        data: [],
        backgroundColor: 'rgba(75, 192, 192, 0.2)',
        borderColor: 'rgba(75, 192, 192, 1)',
        borderWidth: 1
      }]
    },
    options: {
      scales: {
        y: {
          beginAtZero: true
        }
      }
    }
  });
}

function updateDashboard(data) {
  const timestamp = new Date().toLocaleTimeString();
  
  // Update Mbps chart
  mbpsChart.data.labels.push(timestamp);
  mbpsChart.data.datasets[0].data.push(data.stats.mbps);
  mbpsChart.update();
  
  // Update IAT chart
  iatChart.data.labels.push(timestamp);
  iatChart.data.datasets[0].data.push(data.stats.packetcount); // Assuming packetcount represents IAT interval
  iatChart.update();
  
  // Update CC Errors chart
  ccErrorsChart.data.labels.push(timestamp);
  ccErrorsChart.data.datasets[0].data.push(data.stats.ccerrors);
  ccErrorsChart.update();
  
  // Update PID chart
  // Summing up packet counts for each PID as an example
  const totalPacketCountPerPID = data.pids.reduce((acc, pid) => acc + pid.packetcount, 0);
  pidChart.data.labels.push(timestamp);
  pidChart.data.datasets[0].data.push(totalPacketCountPerPID);
  pidChart.update();
  
  // Update Services chart
  // Counting streams for each type as an example
  const serviceTypeCounts = data.services.flatMap(service => service.streams.map(stream => stream.type))
    .reduce((acc, type) => ({ ...acc, [type]: (acc[type] || 0) + 1 }), {});
  servicesChart.data.labels = Object.keys(serviceTypeCounts);
  servicesChart.data.datasets[0].data = Object.values(serviceTypeCounts);
  servicesChart.update();
}
