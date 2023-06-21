// based on https://github.com/ferrous-systems/espressif-trainings/blob/1ec7fd78660c58739019b4c146634077a08e3d5e/common/lib/esp32-c3-dkc02-bsc/src/wifi.rs
// based on https://github.com/ivmarkov/rust-esp32-std-demo/blob/main/src/main.rs

use std::net::Ipv4Addr;
use std::time::Duration;
use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration, Wifi};
use esp_idf_hal::peripheral;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::netif::{EspNetif, EspNetifWait};
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, EspNvsPartition, NvsDefault};
use esp_idf_svc::wifi::{EspWifi, WifiWait};
use heapless::String;
use log::info;
use esp_idf_sys::EspError;
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub enum MyceliumWifiError {
    Esp(EspError),
    Timeout,
    NoIp,
}

#[derive(Serialize, Deserialize)]
pub struct MyceliumWifiSettings {
    pub ssid: String<32>,
    pub password: String<64>,
    pub channel: Option<u8>,
}

pub struct MyceliumWifiEspConnectionResult<R> {
    wifi: R,
    pub channel: Option<u8>,
}

pub trait MyceliumWifi<R> {
    fn find_channel(&mut self, ssid: &String<32>) -> Result<Option<u8>, MyceliumWifiError>;
    fn connect(&mut self, ssid: &String<32>, psk: &String<64>, channel: Option<u8>) -> Result<MyceliumWifiEspConnectionResult<R>, MyceliumWifiError>;
}

pub struct EspMyceliumWifi {
    sysloop: EspSystemEventLoop,
    wifi: EspWifi<'static>,
}

impl EspMyceliumWifi {
    pub fn new(sysloop: EspSystemEventLoop) -> EspMyceliumWifi {
        let modem = unsafe { esp_idf_hal::modem::WifiModem::new() };
        let mut wifi = EspWifi::new(modem, sysloop.clone(), None).unwrap();

        EspMyceliumWifi { sysloop, wifi }
    }
}

impl MyceliumWifi<()> for EspMyceliumWifi {
    fn find_channel(&mut self, ssid: &String<32>) -> Result<Option<u8>, MyceliumWifiError> {
        info!("Searching for WiFi network {}", ssid);

        let ap_infos = self.wifi.scan()?;
        let ours = ap_infos.into_iter().find(|a| a.ssid.eq(ssid));

        if let Some(ours) = ours {
            info!("Found configured access point {} on channel {}", ssid, ours.channel);
            Ok(Some(ours.channel))
        } else {
            info!("Configured access point {} not found during scanning, will go with unknown channel", ssid);
            Ok(None)
        }
    }

    fn connect(&mut self, ssid: &String<32>, psk: &String<64>, channel: Option<u8>) -> Result<MyceliumWifiEspConnectionResult<()>, MyceliumWifiError> {
        let mut auth_method = AuthMethod::WPA2Personal;
        if psk.is_empty() {
            auth_method = AuthMethod::None;
        }

        let preferred_channel = match channel {
            None => self.find_channel(ssid)?,
            Some(v) => Some(v)
        };

        info!("Setting WiFi configuration");
        let conf = Configuration::Client(ClientConfiguration {
            ssid: ssid.clone(),
            password: psk.clone(),
            channel: preferred_channel,
            auth_method,
            ..Default::default()
        });

        self.wifi.set_configuration(&conf)?;
        self.wifi.start()?;

        let wait = WifiWait::new(&self.sysloop)?;

        if !wait.wait_with_timeout(Duration::from_secs(20), || self.wifi.is_started().unwrap()) {
            return Err(MyceliumWifiError::Timeout);
        }

        self.wifi.connect()?;

        if !EspNetifWait::new::<EspNetif>(self.wifi.sta_netif(), &self.sysloop)?.wait_with_timeout(
            Duration::from_secs(10),
            || {
                let connected = self.wifi.is_connected().unwrap();
                let ip_info = self.wifi.sta_netif().get_ip_info().unwrap();

                info!("Status: {:?} {:?}", connected, ip_info);

                connected && ip_info.ip != Ipv4Addr::new(0, 0, 0, 0)
            },
        ) {
            return Err(MyceliumWifiError::NoIp);
        }

        Ok(MyceliumWifiEspConnectionResult { wifi: (), channel: preferred_channel })
    }
}

impl From<EspError> for MyceliumWifiError {
    fn from(value: EspError) -> Self {
        MyceliumWifiError::Esp(value)
    }
}