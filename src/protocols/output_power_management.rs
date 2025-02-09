use std::collections::HashMap;

use smithay::output::Output;
use smithay::reexports::wayland_protocols_wlr::output_power_management::v1::server::{
    zwlr_output_power_manager_v1, zwlr_output_power_v1,
};
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::wayland::{Dispatch2, GlobalDispatch2};
use zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1;
use zwlr_output_power_v1::ZwlrOutputPowerV1;

use crate::protocols::EmptyData;

const VERSION: u32 = 1;

#[derive(Clone, Copy)]
pub enum Mode {
    Off,
    On,
}

impl From<bool> for Mode {
    fn from(value: bool) -> Self {
        match value {
            false => Self::Off,
            true => Self::On,
        }
    }
}

impl From<zwlr_output_power_v1::Mode> for Mode {
    fn from(value: zwlr_output_power_v1::Mode) -> Self {
        match value {
            zwlr_output_power_v1::Mode::Off => Self::Off,
            zwlr_output_power_v1::Mode::On => Self::On,
            _ => unreachable!(),
        }
    }
}

impl From<Mode> for zwlr_output_power_v1::Mode {
    fn from(value: Mode) -> Self {
        match value {
            Mode::Off => Self::Off,
            Mode::On => Self::On,
        }
    }
}

pub struct OutputPowerManagerGlobalData {
    filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

pub trait OutputPowerManagementHandler {
    fn output_power_manager_state(&mut self) -> &mut OutputPowerManagementManagerState;
    fn get_output_power_mode(&mut self, output: &Output) -> Mode;
    fn set_output_power_mode(&mut self, output: &Output, mode: Mode);
}

pub struct OutputPowerManagementManagerState {
    // Active controls only. Failed ones are removed.
    output_powers: HashMap<Output, Vec<ZwlrOutputPowerV1>>,
}

pub struct OutputPowerState {}

impl OutputPowerManagementManagerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<ZwlrOutputPowerManagerV1, OutputPowerManagerGlobalData>,
        D: OutputPowerManagementHandler,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        let global_data = OutputPowerManagerGlobalData {
            filter: Box::new(filter),
        };
        display.create_global::<D, ZwlrOutputPowerManagerV1, _>(VERSION, global_data);
        Self {
            output_powers: HashMap::new(),
        }
    }

    pub fn output_removed(&mut self, output: &Output) {
        if let Some(list) = self.output_powers.remove(output) {
            for num in list {
                num.failed();
            }
        }
    }

    pub fn output_power_mode_changed(&mut self, output: &Output, mode: Mode) {
        if let Some(list) = self.output_powers.get_mut(output) {
            for num in list {
                num.mode(mode.into());
            }
        }
    }
}

impl<D> GlobalDispatch2<ZwlrOutputPowerManagerV1, D> for OutputPowerManagerGlobalData
where
    D: Dispatch<ZwlrOutputPowerManagerV1, EmptyData>,
    D: OutputPowerManagementHandler,
    D: 'static,
{
    fn bind(
        &self,
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        manager: New<ZwlrOutputPowerManagerV1>,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(manager, EmptyData);
    }

    fn can_view(&self, client: &Client) -> bool {
        (self.filter)(client)
    }
}

impl<D> Dispatch2<ZwlrOutputPowerManagerV1, D> for EmptyData
where
    D: Dispatch<ZwlrOutputPowerV1, OutputPowerState>,
    D: OutputPowerManagementHandler,
    D: 'static,
{
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        _resource: &ZwlrOutputPowerManagerV1,
        request: <ZwlrOutputPowerManagerV1 as Resource>::Request,
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwlr_output_power_manager_v1::Request::GetOutputPower { id, output } => {
                let zwlr_output_power = data_init.init(id, OutputPowerState {});
                if let Some(output) = Output::from_resource(&output) {
                    state
                        .output_power_manager_state()
                        .output_powers
                        .entry(output.clone())
                        .or_insert_with(Vec::new)
                        .push(zwlr_output_power.clone());
                    zwlr_output_power.mode(state.get_output_power_mode(&output).into());
                    return;
                }

                // Output not found.
                zwlr_output_power.failed();
            }
            zwlr_output_power_manager_v1::Request::Destroy => (),
            _ => unreachable!(),
        }
    }
}

impl<D> Dispatch2<ZwlrOutputPowerV1, D> for OutputPowerState
where
    D: OutputPowerManagementHandler,
    D: 'static,
{
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        resource: &ZwlrOutputPowerV1,
        request: <ZwlrOutputPowerV1 as Resource>::Request,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            zwlr_output_power_v1::Request::SetMode { mode } => {
                let output_powers = &mut state.output_power_manager_state().output_powers;
                let Some((output, _)) = output_powers.iter().find(|(_, x)| x.contains(resource))
                else {
                    return;
                };
                let output = output.clone();

                if let Ok(mode) = mode.into_result() {
                    // note: if set_output_power_mode becomes fallible, this should
                    // fail the resource; see gamma_control for an example of how to
                    // implement that.
                    state.set_output_power_mode(&output, mode.into());
                } else {
                    resource.failed();
                }
            }
            zwlr_output_power_v1::Request::Destroy => (),
            _ => unreachable!(),
        }
    }

    fn destroyed(&self, state: &mut D, _client: ClientId, resource: &ZwlrOutputPowerV1) {
        let output_powers = &mut state.output_power_manager_state().output_powers;
        output_powers.retain(|_, list| {
            list.retain(|power| power != resource);
            !list.is_empty()
        });
    }
}
