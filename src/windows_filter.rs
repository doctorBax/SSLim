//! Persistent, opt-in IPv4 tunnel guard. Never removed merely by process exit.
//! Only this module's fixed filter keys are modified, in one WFP transaction.
use std::net::Ipv4Addr;
use std::ptr::{null, null_mut};

use anyhow::{Result, ensure};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows_sys::Win32::NetworkManagement::IpHelper::ConvertInterfaceIndexToLuid;
use windows_sys::Win32::NetworkManagement::Ndis::NET_LUID_LH;
use windows_sys::Win32::NetworkManagement::WindowsFilteringPlatform::*;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::core::GUID;

const KEYS: [GUID; 4] = [
    GUID::from_u128(0xc966c5c8_23b4_471a_b088_a51e99591001),
    GUID::from_u128(0xc966c5c8_23b4_471a_b088_a51e99591002),
    GUID::from_u128(0xc966c5c8_23b4_471a_b088_a51e99591003),
    GUID::from_u128(0xc966c5c8_23b4_471a_b088_a51e99591004),
];
const FILTER_NOT_FOUND: u32 = 0x80320003;

/// Serialize all adapter/guard owners, including the emergency unlock command.
pub struct ConnectionLock(HANDLE);
impl ConnectionLock {
    pub fn take() -> Result<Self> {
        let name: Vec<u16> = "Global\\SSLimBladeConnection\0".encode_utf16().collect();
        let handle = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
        let error = unsafe { GetLastError() };
        ensure!(
            handle != 0,
            "could not create Blade connection lock ({error})"
        );
        let lock = Self(handle);
        ensure!(
            error != ERROR_ALREADY_EXISTS,
            "another Blade client is running; stop it before connecting or unlocking"
        );
        Ok(lock)
    }
}
impl Drop for ConnectionLock {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn checked(code: u32) -> Result<()> {
    ensure!(
        code == 0,
        "Windows tunnel guard failed (WFP {code:#x}); administrator rights and BFE are required"
    );
    Ok(())
}

struct Engine(HANDLE);
impl Engine {
    fn open() -> Result<Self> {
        let mut handle = 0;
        // SAFETY: output storage is valid; default local, non-dynamic session.
        unsafe {
            checked(FwpmEngineOpen0(null(), 10, null(), null(), &mut handle))?;
        }
        Ok(Self(handle))
    }

    fn transaction(&self, body: impl FnOnce() -> Result<()>) -> Result<()> {
        // SAFETY: the engine handle is live throughout the transaction.
        unsafe {
            checked(FwpmTransactionBegin0(self.0, 0))?;
        }
        let result = body().and_then(|()| unsafe { checked(FwpmTransactionCommit0(self.0)) });
        if result.is_err() {
            unsafe {
                FwpmTransactionAbort0(self.0);
            }
        }
        result
    }

    fn remove(&self) -> Result<()> {
        for key in &KEYS {
            let status = unsafe { FwpmFilterDeleteByKey0(self.0, key) };
            if status != FILTER_NOT_FOUND {
                checked(status)?;
            }
        }
        Ok(())
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        unsafe {
            FwpmEngineClose0(self.0);
        }
    }
}

pub fn clear() -> Result<()> {
    let engine = Engine::open()?;
    engine.transaction(|| engine.remove())
}

fn ranges(server: u32) -> Vec<(u32, u32)> {
    let mut ranges = Vec::new();
    if server > 0 {
        ranges.push((0, server - 1));
    }
    // Preserve DHCP broadcast so a physical adapter can acquire a new lease.
    if server < u32::MAX - 1 {
        ranges.push((server + 1, u32::MAX - 1));
    }
    ranges
}

/// Endpoint is a pinned IPv4 literal; SNI remains independently configurable.
/// Index None blocks all ordinary outbound traffic until Wintun is ready.
pub fn install(server: Ipv4Addr, index: Option<u32>) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let executable = std::env::current_exe()?;
    let filename: Vec<u16> = executable
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut app_id = null_mut();
    unsafe {
        checked(FwpmGetAppIdFromFileName0(filename.as_ptr(), &mut app_id))?;
    }
    struct AppId(*mut FWP_BYTE_BLOB);
    impl Drop for AppId {
        fn drop(&mut self) {
            unsafe {
                FwpmFreeMemory0((&mut self.0 as *mut *mut FWP_BYTE_BLOB).cast());
            }
        }
    }
    let app_id = AppId(app_id);
    let mut luid: NET_LUID_LH = unsafe { std::mem::zeroed() };
    if let Some(index) = index {
        unsafe {
            checked(ConvertInterfaceIndexToLuid(index, &mut luid))?;
        }
    }
    let mut interface = unsafe { luid.Value };
    let engine = Engine::open()?;
    engine.transaction(|| {
        engine.remove()?;
        for (i, (low, high)) in ranges(u32::from(server)).into_iter().enumerate() {
            let mut range = FWP_RANGE0 {
                valueLow: FWP_VALUE0 {
                    r#type: FWP_UINT32,
                    Anonymous: FWP_VALUE0_0 { uint32: low },
                },
                valueHigh: FWP_VALUE0 {
                    r#type: FWP_UINT32,
                    Anonymous: FWP_VALUE0_0 { uint32: high },
                },
            };
            let mut conditions = vec![
                FWPM_FILTER_CONDITION0 {
                    fieldKey: FWPM_CONDITION_IP_REMOTE_ADDRESS,
                    matchType: FWP_MATCH_RANGE,
                    conditionValue: FWP_CONDITION_VALUE0 {
                        r#type: FWP_RANGE_TYPE,
                        Anonymous: FWP_CONDITION_VALUE0_0 {
                            rangeValue: &mut range,
                        },
                    },
                },
                FWPM_FILTER_CONDITION0 {
                    fieldKey: FWPM_CONDITION_IP_LOCAL_INTERFACE,
                    matchType: FWP_MATCH_NOT_EQUAL,
                    conditionValue: FWP_CONDITION_VALUE0 {
                        r#type: FWP_UINT64,
                        Anonymous: FWP_CONDITION_VALUE0_0 {
                            uint64: &mut interface,
                        },
                    },
                },
                no_loopback(),
            ];
            add(
                &engine,
                KEYS[i],
                FWPM_LAYER_OUTBOUND_IPPACKET_V4,
                &mut conditions,
            )?;
        }
        // The current Blade data path is IPv4-only. Block external IPv6 fully.
        add(
            &engine,
            KEYS[2],
            FWPM_LAYER_OUTBOUND_IPPACKET_V6,
            &mut [no_loopback()],
        )?;
        // The packet-layer endpoint exception is restricted at ALE to this
        // executable, so other programs cannot send DNS/data directly to it.
        let mut conditions = [
            FWPM_FILTER_CONDITION0 {
                fieldKey: FWPM_CONDITION_IP_REMOTE_ADDRESS,
                matchType: FWP_MATCH_EQUAL,
                conditionValue: FWP_CONDITION_VALUE0 {
                    r#type: FWP_UINT32,
                    Anonymous: FWP_CONDITION_VALUE0_0 {
                        uint32: u32::from(server),
                    },
                },
            },
            FWPM_FILTER_CONDITION0 {
                fieldKey: FWPM_CONDITION_IP_LOCAL_INTERFACE,
                matchType: FWP_MATCH_NOT_EQUAL,
                conditionValue: FWP_CONDITION_VALUE0 {
                    r#type: FWP_UINT64,
                    Anonymous: FWP_CONDITION_VALUE0_0 {
                        uint64: &mut interface,
                    },
                },
            },
            FWPM_FILTER_CONDITION0 {
                fieldKey: FWPM_CONDITION_ALE_APP_ID,
                matchType: FWP_MATCH_NOT_EQUAL,
                conditionValue: FWP_CONDITION_VALUE0 {
                    r#type: FWP_BYTE_BLOB_TYPE,
                    Anonymous: FWP_CONDITION_VALUE0_0 { byteBlob: app_id.0 },
                },
            },
        ];
        add(
            &engine,
            KEYS[3],
            FWPM_LAYER_ALE_AUTH_CONNECT_V4,
            &mut conditions,
        )
    })
}

fn no_loopback() -> FWPM_FILTER_CONDITION0 {
    FWPM_FILTER_CONDITION0 {
        fieldKey: FWPM_CONDITION_FLAGS,
        matchType: FWP_MATCH_FLAGS_NONE_SET,
        conditionValue: FWP_CONDITION_VALUE0 {
            r#type: FWP_UINT32,
            Anonymous: FWP_CONDITION_VALUE0_0 {
                uint32: FWP_CONDITION_FLAG_IS_LOOPBACK,
            },
        },
    }
}

fn add(
    engine: &Engine,
    key: GUID,
    layer: GUID,
    conditions: &mut [FWPM_FILTER_CONDITION0],
) -> Result<()> {
    let mut name: Vec<u16> = "Blade persistent tunnel guard\0".encode_utf16().collect();
    let filter = FWPM_FILTER0 {
        filterKey: key,
        displayData: FWPM_DISPLAY_DATA0 {
            name: name.as_mut_ptr(),
            description: null_mut(),
        },
        flags: FWPM_FILTER_FLAG_PERSISTENT,
        layerKey: layer,
        subLayerKey: FWPM_SUBLAYER_UNIVERSAL,
        numFilterConditions: conditions.len() as u32,
        filterCondition: conditions.as_mut_ptr(),
        action: FWPM_ACTION0 {
            r#type: FWP_ACTION_BLOCK,
            ..unsafe { std::mem::zeroed() }
        },
        ..unsafe { std::mem::zeroed() }
    };
    // SAFETY: WFP copies all pointed-to data synchronously during this call.
    unsafe { checked(FwpmFilterAdd0(engine.0, &filter, null_mut(), null_mut())) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_endpoint_and_broadcast_are_excluded() {
        for endpoint in [0, 1, 0x01020304, u32::MAX - 1, u32::MAX] {
            let blocks = ranges(endpoint);
            for ip in [
                0,
                1,
                endpoint.saturating_sub(1),
                endpoint,
                endpoint.saturating_add(1),
                u32::MAX - 1,
                u32::MAX,
            ] {
                assert_eq!(
                    blocks.iter().any(|&(lo, hi)| (lo..=hi).contains(&ip)),
                    ip != endpoint && ip != u32::MAX
                );
            }
        }
    }
}
