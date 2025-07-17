# Introduction
Platform service provides generic abstraction over embedded functions so implementation can remain hardware agnostic.

Services contained:
- CRC

Additional usage examples can be found in embedded-services/examples/rt685s-evk

# CRC
Contains EmbeddedCrc struct which can be instantiated to calculate a CRC over a specific algorithm.
EmbeddedCrc internally keeps track of the state of the CRC calculation, which supports calculations that are split into multiple calls.
EmbeddedCrc leverages the Algorithm consts defined in the Rust CRC package: https://docs.rs/crc-catalog/2.4.0/src/crc_catalog/algorithm.rs.html
Most of the structure is re-used across different controllers. The embedded implementation used - which is toggled via cargo feature flags - selects the implementation of the crc_calculate_\[type\]() methods. While the default software CRC implementation does not need resource guarding, this method also contains the mutex for guarding limited hardware registers.
Depending on the usage, an internal CRC hardware object may need initialization at the top of a program. Afterwards, the user is free instantiate a new EmbeddedCrc object for each new CRC calculation needed without needing to pass any more resources down to the calculation location.

## Public Structure
```
struct EmbeddedCrc<W: crc::Width>
    fn new(algorithm: &'static Algorithm<W>) -> Self
    async fn calculate(&mut self, bytes: &[u8]) -> Result<W, EmbeddedCrcError>
    fn read_crc(&self) -> W

enum EmbeddedCrcError {
    #[default]
    CrcErrorUnknown,
    CrcErrorWidth,
    CrcErrorPolynomial,
    CrcErrorXorOut,
    CrcErrorMutexGet,
}

(IMXRT Only - Hardware Initialization)
fn crc_engine_init(crc: embassy_imxrt::crc::Crc<'static>)
```




## Usage

### Hardware Initialization (IMXRT)
main.rs: Initialize the hardware CRC object with the Embassy IMXRT peripherals
```
let p: embassy_imxrt::Peripherals = embassy_imxrt::init(Default::default());
let crc: embassy_imxrt::crc::Crc<'_> = embassy_imxrt::crc::Crc::new(p.CRC, Default::default());
platform_service::imxrt::embedded_crc::crc_engine_init(crc);
```

### Carrying out a CRC calculation
Create an EmbeddedCrc object with your desired algorithm. Let's do a 32-bit Checksum calculation, for instance
The algorithm structure comes from the CRC catalog: `&CRC_32_CKSUM`
`let mut crc_handle: embedded_crc::EmbeddedCrc<u32> = embedded_crc::EmbeddedCrc::<u32>::new(&CRC_32_CKSUM);`

Calculate the CRC across a slice of data &[u8] using the `calculate()` method. This method returns the value of the CRC after that calculation.
```
let crc = match crc_handle.calculate(DATA).await {
    Ok(crc) => crc,
    Err(e) => {
        error!("Error calculating CRC32 for {}: {}", name, e);
        return;
    }
};
```

Split calculations can be made by calling the `calculate()` method again on the next set of data. The internal state of the CRC calculation is kept within each object.
For instance, the following code leaves `crc` in the same state as the above one-shot calculation:
```
let _crc = match crc_handle.calculate(&DATA[0..(DATA.len() / 2)]).await { (error handling) };
let crc  = match crc_handle.calculate(&DATA[(DATA.len() / 2)..]).await { (error handling) };
```

In order to start over with either a new algorithm, or to reset the algorithm to a new calculation, simply make another EmbeddedCrc object:
`let mut crc_handle_2: embedded_crc::EmbeddedCrc<u16> = embedded_crc::EmbeddedCrc::<u16>::new(&CRC_16_XMODEM);`