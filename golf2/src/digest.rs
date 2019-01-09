// Copyright 2018 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use core::cell::Cell;
use h1b::hil::digest::{DigestEngine, DigestError, DigestMode};
use kernel::{AppId, AppSlice, Driver, Grant, ReturnCode, Shared};

pub const DRIVER_NUM: usize = 0x40003;

/// Per-application driver data.
pub struct App {
    /// Buffer where data to be hashed will be read from.
    input_buffer: Option<AppSlice<Shared, u8>>,
    /// Buffer where the digest will be written to when hashing is finished.
    output_buffer: Option<AppSlice<Shared, u8>>,
}

impl Default for App {
    fn default() -> App {
        App {
            input_buffer: None,
            output_buffer: None,
        }
    }
}

pub struct DigestDriver<'a, E: DigestEngine + 'a> {
    engine: &'a E,
    apps: Grant<App>,
    current_user: Cell<Option<AppId>>,
}

impl<'a, E: DigestEngine + 'a> DigestDriver<'a, E> {
    pub fn new(engine: &'a E, container: Grant<App>) -> DigestDriver<'a, E> {
        DigestDriver {
            engine: engine,
            apps: container,
            current_user: Cell::new(None),
        }
    }
}

impl<'a, E: DigestEngine> Driver for DigestDriver<'a, E> {
    fn command(&self, minor_num: usize, r2: usize, _r3: usize, caller_id: AppId) -> ReturnCode {
        match minor_num {
            // Initialize hash engine (arg: digest mode)
            0 => {
                self.apps
                    .enter(caller_id, |app_data, _| {
                        if self.current_user.get().is_some() {
                            return ReturnCode::EBUSY;
                        }
                        self.current_user.set(Some(caller_id));

                        let digest_mode = match r2 {
                            0 => DigestMode::Sha1,
                            1 => DigestMode::Sha256,
                            2 => DigestMode::Sha256Hmac,
                            _ => return ReturnCode::EINVAL,
                        };
                        let init_result = match digest_mode {
                            DigestMode::Sha1 | DigestMode::Sha256 =>
                                self.engine.initialize(digest_mode),
                            DigestMode::Sha256Hmac => {
                                let input_buffer = match app_data.input_buffer {
                                    Some(ref slice) => slice,
                                    None => return ReturnCode::ENOMEM
                                };
                                self.engine.initialize_hmac(&input_buffer.as_ref())
                            }
                        };
                        match init_result {
                            Ok(_t) => return ReturnCode::SUCCESS,
                            Err(DigestError::EngineNotSupported) => return ReturnCode::ENOSUPPORT,
                            Err(DigestError::NotConfigured) => return ReturnCode::FAIL,
                            Err(DigestError::BufferTooSmall(_s)) => return ReturnCode::ESIZE
                        }
                    }).unwrap_or(ReturnCode::ENOMEM)
            },
            // Feed data from input buffer (arg: number of bytes)
            1 => {
                self.apps
                    .enter(caller_id, |app_data, _| {
                        match self.current_user.get() {
                                Some(cur) if cur.idx() == caller_id.idx() => {}
                            _ => {
                                return ReturnCode::EBUSY
                            }
                        }
                        let app_data: &mut App = app_data;

                        let input_buffer = match app_data.input_buffer {
                            Some(ref slice) => slice,
                            None => return ReturnCode::ENOMEM
                        };
                        let input_len = r2;
                        if input_len > input_buffer.len() {
                            return ReturnCode::ESIZE
                        }
                        match self.engine.update(&input_buffer.as_ref()[..input_len]) {
                            Ok(_t) => ReturnCode::SUCCESS,
                            Err(DigestError::EngineNotSupported) => ReturnCode::ENOSUPPORT,
                            Err(DigestError::NotConfigured) => ReturnCode::ERESERVE,
                            Err(DigestError::BufferTooSmall(_s)) => ReturnCode::ESIZE
                        }
                    })
                    .unwrap_or(ReturnCode::ENOMEM)
            },
            // Finalize hash and output to output buffer (arg: unused)
            2 => {
                self.apps
                    .enter(caller_id, |app_data, _| {
                        match self.current_user.get() {
                            Some(cur) if cur.idx() == caller_id.idx() => {}
                            _ => {
                                return ReturnCode::EBUSY
                            }
                        }
                        self.current_user.set(None);
                        let app_data: &mut App = app_data;

                        let output_buffer = match app_data.output_buffer {
                            Some(ref mut slice) => slice,
                            None => return ReturnCode::ENOMEM
                        };

                        match self.engine.finalize(output_buffer.as_mut()) {
                            Ok(_t) => ReturnCode::SUCCESS,
                            Err(DigestError::EngineNotSupported) => ReturnCode::ENOSUPPORT,
                            Err(DigestError::NotConfigured) => ReturnCode::FAIL,
                            Err(DigestError::BufferTooSmall(_s)) => ReturnCode::ESIZE
                        }

                    })
                    .unwrap_or(ReturnCode::ENOMEM)
            },
            _ => ReturnCode::ENOSUPPORT
        }
    }

    fn allow(&self,
             app_id: AppId,
             allow_num: usize,
             slice: Option<AppSlice<Shared, u8>>
    ) -> ReturnCode {
        match allow_num {
                0 => {
                    // Input buffer
                    self.apps
                        .enter(app_id, |app_data, _| {
                            app_data.input_buffer = slice;
                            ReturnCode::SUCCESS
                        })
                        .unwrap_or(ReturnCode::ENOMEM)
                }
                1 => {
                    // Hash output buffer
                    self.apps
                        .enter(app_id, |app_data, _| {
                            app_data.output_buffer = slice;
                            ReturnCode::SUCCESS
                        })
                        .unwrap_or(ReturnCode::ENOMEM)
                }
                _ => ReturnCode::ENOSUPPORT,
            }
    }
}
