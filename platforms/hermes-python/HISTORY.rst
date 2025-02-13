## History
### 0.2.0 (2019-02-04)
* Introduces options to connect to the MQTT broker (auth + TLS are now supported).

### 0.1.29 (2019-01-29)
* Fixes bug when deserializing `TimeIntervalValue` that used wrong `encode` method instead of `decode`.


### 0.1.28 (2019-01-14)
* Fixes bug when the `__exit__` method was called twice on the `Hermes` class.
* Introduces two methods to the public api : `connect` and `disconnect` that should bring more flexibility

### 0.1.27 (2019-01-07)
* Fixed broken API introduced in `0.1.26` with the publish_continue_session method of the Hermes class. 
* Cast any string that goes in the mqtt_server_adress parameter in the constructor of the Hermes class to be a 8-bit string.

### 0.1.26 (2019-01-02)
* LICENSING : This wheel now has the same licenses as the parent project : APACHE-MIT. 
* Subscription to not recognized intent messages is added to the API. You can now write your own callbacks to handle unrecognized intents.  
* Adds send_intent_not_recognized flag to continue session : indicate whether the dialogue manager should handle non recognized intents by itself or sent them as an `IntentNotRecognizedMessage` for the client to handle.

### 0.1.25 (2018-12-13)
* Better error handling : Errors from wrapped C library throw a LibException with detailled errors. 


