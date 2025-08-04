let binding = require("../packages/binding");

// Setup logging
let [logger, doneLogging] = binding.configureLogger((err, val) => {
  if (err) {
    console.err(err);
  } else {
    console.log(val);
  }
});

// Setup device listener
let [devices, doneListening] = binding.listen((err, val) => {
  if (err) {
    console.err(err);
  } else {
    console.log(val);
  }
});

// We're done logging
doneLogging.then(() => console.log("closed logger"));
doneListening.then(() => console.log("closed devices"));

setTimeout(() => {
  console.log("ending demo");
  logger.abort();
  devices.abort();
}, 15000);
