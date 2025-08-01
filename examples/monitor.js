let binding = require("../packages/bindings");
let monitor = new binding.Monitor();
(async function () {
  let stream = monitor.listen();
  console.log(stream);
  for await (const chunk of stream) {
    console.log(stream);
    console.log(chunk);
  }
})();
setTimeout(() => monitor.abort(), 5000);
