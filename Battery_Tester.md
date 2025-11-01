# Objectives

System Design for the battery tester.

# Concept Study

We want to know if a battery will perform adequately in a match.
The best way we are aware of to do this is check if it preforms similar to its own specification.

We can check how long a battery takes to reach a "discharged" state (as we define it) under a load its manufacturer considers typical.

Cutoff voltage should be no less than 11 volts.

We typically don't care about voltage differences less than 10 mV.

In order to know the load, we will need to measure the current periodically and average it over the test time.
The measurement interval shall be such that the battery voltage won't drop more than 10mV between measurements under test load.

# Operational Analysis

Basic scenario:

1. Charge the battery
1. Wait 5 minutes
1. Record battery ID
1. Record test start time
1. Connect to ~10A load
1. Record actual current @ 10Hz
1. Disconnect when at cutoff voltage
1. Start charging battery
1. Calculate Amp/Hour rating using average

Mid-test stop:

1. Disconnect battery
1. Delete recording of incomplete test

# System Needs Analysis

SUD shall be responsible for:

- Data acquisition
- Data persistence
- Connecting the battery to the load
- Knowing when to end the test
- Disconnecting the battery from the load

Basic scenario:

1. Record battery ID
1. Record test start time
1. Connect to ~10A load
1. Record current at 10Hz
1. Disconnect battery from load when at cutoff voltage

# Logical Architecture

## Tester Components

* Data recorder
* DAQ
	* Current sensor
	* Voltage sensor
* Load; 10A - 12A @ 12V - 11V 
* Load Controller: needs to switch 12V at up to 15A.
* Manual Battery Disconnect
	* Connector cable
	* Main breaker

PC components:

* Data recording
* DAQ control (weather to transmit data)
* Load control

## Basic Process

1. Tester waits for user to enter battery ID
1. Tester waits for battery connection
	1. This may happen before battery ID but we only check for it after
1. Tester waits for user to start test
1. Tester connects battery to load
1. Tester records current & voltage readings
1. After battery is below cutoff, tester disconnects load
1. Tester notifies user that the test is complete


## States

Initial state is [Wait For ID](#wait-for-id).

### Wait For ID

Wait for user to enter the battery ID.

Next states:

- [Wait for Battery](#wait-for-battery): user entered battery ID

### Wait for Battery

Wait for the user to connect the battery.

Next states:

- [Wait for ID](#wait-for-id): user cancels test
- [Wait for start](#wait-for-start): system detects battery voltage above cutoff.

### Wait for start

Wait for the user to start the test.

Next states:

- [Wait For ID](#wait-for-id): user cancels test
- [Battery Disconnect](#battery-disconnect): system detects voltage < 1 volt
- [Testing](#testing): user starts test

### Battery Disconnect

1. Cut power to load.
1. Warn that battery is disconnected

Next states:

- [Wait For ID](#wait-for-id): User acknowledge fault

### Testing

1. Turn on load 
2. Log voltage and current data

Next states:

- [Battery Disconnect](#battery-disconnect): system detects voltage < 1 volt
- [Paused](#paused): user pauses test
- [End Test](#end-test): user cancels test
- [End Test](#end-test): system detects that battery voltage is less than or equal to cutoff voltage

### Paused

1. Turn off load
2. Stop logging voltage and current

Next states:

- [Battery Disconnect](#battery-disconnect): system detects voltage < 1 volt
- [End Test](#end-test): user cancels test

### End Test

1. Stop logging voltage and current
1. Cut power to load
1. Save all data

Next states:

- [Wait for ID](#wait-for-id): auto

## Refinement

We know that to get an average of the current measurements we need to store all of them so a PC (Rpi or larger) is needed.
The PC would need an interface to the hardware, Battery Interface (BI).
The PC would have more overhead to handle whatever control logic while the BI would translate that into lower level signals.

BI will translate PC commands into commands to the Load Controller and send back readings from the Current and Voltage sensors.
BI and PC will be connected by USB or Serial (over USB) as this is the only option available on most PCs.

## Requirements

* The BI shall turn off the heater if it detects lower than expected current.
* The BI shall turn off the heater if it detects higher than expected current.
* The BI shall turn off the heater if it doesn't receive a control signal from the PC at 1Hz or more often.
* The BI shall collect measurements at 10Hz.
* The BI shall average every 10 measurements.
* The BI shall store the average of the most recent 10 measurements.
* The BI shall use the most recent measurement for checking heater current.
* The BI shall how many milliseconds since heater state changed from off to on.
* The BI shall Check that battery is connected before attempting I2C communication.

# Physical Architecture
 Collect data while on.
Update heater on every command received.

On every 10Hz interval:

1. Check that battery is connected before attempting I2C comm
1. Check that current is in expected range
1. Check how long its been since the last command
1. Update the DAQ queue
1. Read most recent command

On every command:

1. Update heater output
1. Update command time
1. Send whatever the command asks for

When heater is turned on reset and start the test start time.
When command asks for time, send how long its been since the test start time. 

## Software

Two tasks, one for handling DAQ and one for PC comm.
Both tasks share access to PWM so either can turn it off in the same loop.

## Hardware

* PWM motor controller
* 5V supply
* I2C isolator
* Opto-isolator (for PWM)
* INA260
