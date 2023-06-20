// based on https://github.com/ferrous-systems/espressif-trainings/blob/1ec7fd78660c58739019b4c146634077a08e3d5e/common/lib/esp32-c3-dkc02-bsc/src/wifi.rs
// based on https://github.com/ivmarkov/rust-esp32-std-demo/blob/main/src/main.rs

use std::net::Ipv4Addr;
use std::time::Duration;
use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration, Wifi};
use esp_idf_hal::peripheral;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::netif::{EspNetif, EspNetifWait};
use esp_idf_svc::wifi::{EspWifi, WifiWait};
use heapless::String;
use log::info;
use esp_idf_sys::EspError;

#[derive(Debug)]
pub enum WifiError {
    Esp(EspError),
    Timeout,
    NoIp
}

pub fn wifi(
    ssid: &String<32>,
    psk: &String<64>,
    sysloop: EspSystemEventLoop
) -> Result<EspWifi<'static>, WifiError> {
    let mut auth_method = AuthMethod::WPA2Personal;
    if psk.is_empty() {
        auth_method = AuthMethod::None;
    }

    let modem = unsafe { esp_idf_hal::modem::WifiModem::new() };
    let mut wifi = EspWifi::new(modem, sysloop.clone(), None).map_err(|err| WifiError::Esp(err))?;
    info!("Searching for WiFi network {}", ssid);

    let ap_infos = wifi.scan().map_err(|err| WifiError::Esp(err))?;
    let ours = ap_infos.into_iter().find(|a| a.ssid.eq(ssid));
    let channel = if let Some(ours) = ours {
        info!(
            "Found configured access point {} on channel {}",
            ssid, ours.channel
        );
        Some(ours.channel)
    } else {
        info!(
            "Configured access point {} not found during scanning, will go with unknown channel",
            ssid
        );
        None
    };

    info!("Setting WiFi configuration");
    let conf = Configuration::Client(ClientConfiguration {
        ssid: ssid.clone(),
        password: psk.clone(),
        channel,
        auth_method,
        ..Default::default()
    });

    wifi.set_configuration(&conf).map_err(|err| WifiError::Esp(err))?;

    info!("Getting WiFi status");
    wifi.start().map_err(|err| WifiError::Esp(err))?;

    let wait = WifiWait::new(&sysloop).map_err(|err| WifiError::Esp(err))?;

    if !wait.wait_with_timeout(Duration::from_secs(20), || wifi.is_started().unwrap()) {
        return Err(WifiError::Timeout)
    }

    info!("Started");

    wifi.connect().map_err(|err| WifiError::Esp(err))?;

    if !EspNetifWait::new::<EspNetif>(wifi.sta_netif(), &sysloop)
        .map_err(|err| WifiError::Esp(err))?
        .wait_with_timeout(
        Duration::from_secs(10),
        || {
            let connected= wifi.is_connected().unwrap();
            let ip_info =  wifi.sta_netif().get_ip_info().unwrap();

            info!("Status: {:?} {:?}", connected, ip_info);

            connected && ip_info.ip != Ipv4Addr::new(0, 0, 0, 0)
        },
    ) {
        return Err(WifiError::NoIp)
    }

    Ok(wifi)
}
