[![check](https://github.com/OpenDevicePartnership/embedded-services/actions/workflows/check.yml/badge.svg)](https://github.com/OpenDevicePartnership/embedded-services/actions/workflows/check.yml)
[![no-std](https://github.com/OpenDevicePartnership/embedded-services/actions/workflows/nostd.yml/badge.svg)](https://github.com/OpenDevicePartnership/embedded-services/actions/workflows/nostd.yml)
[![rolling](https://github.com/OpenDevicePartnership/embedded-services/actions/workflows/rolling.yml/badge.svg)](https://github.com/OpenDevicePartnership/embedded-services/actions/workflows/rolling.yml)
[![LICENSE](https://img.shields.io/badge/License-MIT-blue)](./LICENSE)

# EC Services

## Overview

EC service is where the business logic glues the HAL + common EC functional traits + EC peripheral driver together.

## Building Blocks

### MCU Platform HAL

Hardware specific HAL leveraging Rust Async framework

- Must implement embedded-hal traits to allow a generic hardware agnostic interface
- Desire is for HALs to be open-sourced and upstreamed to Embassy main repo
- Plan to partner with MCU vendor to support more MCU in the future

For example, [embassy-imxrt](https://github.com/OpenDevicePartnership/embassy-imxrt)

```mermaid
        classDiagram
            class embassy-imxrt["embassy-imxrt I2C master HAL"]
            class embedded-hal["embedded-hal I2C master traits"]
            <<interface>> embedded-hal
            embedded-hal <|-- embassy-imxrt
            embedded-hal: +read()
            embedded-hal: +write()
            embassy-imxrt: +read()
            embassy-imxrt: +write()
```

### EC Subsystem Platform Abstractions

There are sets of generic Rust traits the define an EC functional subsystem like thermal, USB PD, fan, battery. This abstraction serves to abstract the underlying HW design away from the business logic.

For example, [embedded-sensor](https://github.com/OpenDevicePartnership/embedded-sensors)

```mermaid
    classDiagram
        embedded-sensor: +temperature()
        <<interface>> embedded-sensor
```

### Rust Based Drivers for EC Peripherals

There are MCU platform agnostic Rust drivers for specific HW parts connected to the EC like TMP108 temp sensor.

- Depending on embedded-hal interface so it is talking to a generic HW interface, not tying to any specific MCU platform
- Implements a EC function platform abstraction traits like `embedded-sensor`, `embedded-battery`, `embedded-fan`
- Plan to partner with vendor to open-source these drivers

For example, [tmp108](https://github.com/OpenDevicePartnership/tmp108)

```mermaid
    classDiagram
        embedded-sensor <|-- TMP108
        embedded-sensor: +temperature()
        <<interface>> embedded-sensor
        TMP108 --> embedded-hal
        TMP108: +temperature()
        class embassy-imxrt["embassy-imxrt I2C master HAL"]
        class embedded-hal["embedded-hal I2C master traits"]
        <<interface>> embedded-hal
        embedded-hal <|-- embassy-imxrt
        embedded-hal: +read()
        embedded-hal: +write()
        embassy-imxrt: +read()
        embassy-imxrt: +write()
```

## EC Services Repo

EC service houses the business logic that glues the EC peripheral Rust driver + EC subsystem platform abstraction + MC platform HAL together/.

## Repo Organization

- embedded-services repo
  - embedded-services library crate
    - service traits
    - intrusive-list
    - transport/router
- power-button-service
  - library crate
- hid-service
  - library crate
- cfu-service
  - library crate
    - host traits
    - client traits

### embedded-services

This houses common EC service utilities to build a service. It includes:

- instrusive-list that allows dynamic number of subscribers and publishers for a service
- transport (IPC) logic that allows EC services to talk to each other

#### transport (IPC)

Protocol agnostric transport allowing dynamic number of endpoints to send and receive message. It makes use of the instrusive list to allow dynamic number of endpoints corresponding to each endpoint ID.

```mermaid
    erDiagram
        service_a ||..|| endpoint_a :contains
        endpoint_a ||..|| transport : send_message
        transport ||--|| endpoint_b : route
        service_b ||..|| endpoint_b :contains
        transport {
            list endpoint_a
            list endpoint_b
            list endpoint_c
        }

```

Message are opaque pointer to a Rust object, so it literally can be anything.

### Individual services

Services will be separate crates in this repo. Each service crate will be implementation of the interfaces for a functional area.

The service itself should be hardware/platform agnostic and contains the application logic for EC functionality.

For example, temperature_service

```mermaid
    classDiagram
        temperature-service --> embedded-hal
        temperature-service --> embedded-sensor
        embedded-sensor <|-- TMP108
        embedded-sensor: +temperature()
        <<interface>> embedded-sensor
        TMP108 --> embedded-hal
        TMP108: +temperature()
        class embassy-imxrt["embassy-imxrt I2C master HAL"]
        class embedded-hal["embedded-hal I2C master traits"]
        <<interface>> embedded-hal
        embedded-hal <|-- embassy-imxrt
        embedded-hal: +read()
        embedded-hal: +write()
        embassy-imxrt: +read()
        embassy-imxrt: +write()
```

#### hid-service

HID over I2c transport

```mermaid
    classDiagram
        keyboard-service --> embedded-keyboard-rs
        keyboard-service --> embedded-hid
        touchpad-service --> passthrough-service
        passthrough-service --> embedded-hid
        passthrough-service --> i2c-host-service
        passthrough-service --> i2c-device-service
```

- keyboard-service
  - embedded-keyboard-rs
  - embedded-hid

- touchpad-service
  - passthrough-service
    - embedded-hid
    - i2c-host-service
    - i2c-device-service



#### power-button-service

Service to manage a power button

#### espi-service

Provide eSPI transport, similar to traditional x86 EC, a memory map table of information will be maintained.

```mermaid
   erDiagram
    p[MemoryTable] {
        u32 battery_status
        u32 battery_charge_threshold
        u32 field
    }
```

##### read operation

```mermaid
    sequenceDiagram
        battery_service->>espi_service: Update battery status
        host-->>espi_service: Get battery status
        espi_service-->>host: Provide cached battery status
        battery_service->>espi_service: Update battery status
```

- service register with the espi service for an entry in the table
- service periodically update their table entries by sending a message throught transport to espi_service
- host eSPI peripheral channel read always gets the cached value

##### write operation

```mermaid
    sequenceDiagram
        host-->>espi_service: Set battery charge threshold
        espi_service->>battery_service: Set battery charge threshold
        battery_service-->>charger: Set battery charge threshold
        battery_service->>espi_service: Done setting battery charge threshold
        espi_service-->>host: Battery charge threshold updated
```

- service register with the espi service for an entry in the table
- host eSPI peripherla channel write opertions triggers espi_service to send message to battery_service to update the charge threshold
- battery service performs the bus operation to update the charge threshold on the charge
- after bus operation is done, battery service notifies espi_service
- espi_service updates the memory table and optionally can notify the host

#### nvm-service (planned)

```mermaid
    classDiagram
        nvm-service --> embedded-storage
        embedded-storage <|-- FlexSPI
        embedded-storage <|-- SPI

        <<interface>> embedded-storage
        class FlexSPI["embassy-imxrt FlexSPI HAL"]
        class SPI["embassy-imxrt SPI HAL"]
```

- nvm-service
  - embedded-nvm (generic, would prefer to have filesystem features like error checking and file structures)
    - embedded-storage
      - FlexSPI
      - SPI

#### rtc-service (planned)

```mermaid
    classDiagram
        rtc-service --> embedded-rtc
        embedded-rtc <|-- RTC
        <<interface>> embedded-rtc
        class RTC["embassy-imxrt RTC HAL"]
```

- rtc-service
  - embedded-rtc 
    - embassy-imxrt RTC HAL

#### thermal-service (planned)

```mermaid
    classDiagram
        thermal-service --> embedded-fan
        thermal-service --> embedded-sensor
        embedded-sensor <|-- sensor-driver
        embedded-fan <|-- fan-driver
        <<interface>> embedded-fan
        <<interface>> embedded-sensor
        sensor-driver --> embedded-hal
        fan-driver --> embedded-timer
        <<interface>> embedded-hal
        <<interface>> embedded-timer
```

- thermal-service
  - embedded-sensor
    - product specific sensor driver
  - embedded-fan
    - product specific fan driver

#### cfu-service (planned)

```mermaid
    classDiagram
        cfu-service --> embedded-cfu
        cfu-service --> embedded-hal
        cfu-service --> embedded-storage
```

- cfu-service
  - embedded-cfu
  - embedded-hal
  - embedded-storage

#### battery-service (planned)

```mermaid
    classDiagram
        battery-service --> embedded-battery
        battery-service --> embedded-charger
```

- battery-service
  - embedded-battery
  - embedded-charger

#### usb-c-service (planned)

```mermaid
    classDiagram
        usb-c-service --> embedded-usb-pd
```


- usb-c-service
  - embedded-usb-pd

## EC Top-Level

At the top-level, a EC is an aggregate of service.

Sets of services can be grouped into subsystem. For instance, thermal subsystem will consist of temperature-service + fan-service + battery-service + debug-service + host-comm-service. The service talks to each other through the transport (IPC) layer. An EC service can also be shared between different subsystems. For instance, debug-service will subcribe to debug messages from other services.

``` Rust
async fn (spawner: Spawner) {
    //initialize HW peripheral and system level managemetn
    spawn(services(periphal, configuration))
    ...
}
```

### Example: Simplified Layer of Subsystem + Services

![Simplified Layer View](docs/images/simplified_layers.png "Simplified Layered View of Services")

### Example: E2E of Keyboard over eSPI

![Keyboard to Host via eSPI Example](docs/images/keyboard_to_espi_example.png "Keyboard to Host via eSPI Example")
