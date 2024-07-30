/*
 * Copyright (C) 2024 Open Source Robotics Foundation
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 *
*/

use super::*;
use bevy::{ecs::system::SystemParam, prelude::*};
use bevy_egui::egui::{
    Button, CollapsingHeader, Color32, ComboBox, DragValue, Frame, Grid as EguiGrid, ScrollArea,
    Slider, Stroke, Ui,
};
use rmf_site_editor::{
    interaction::{Select, Selection},
    occupancy::Grid,
    site::{
        Category, Change, ChangeCurrentScenario, CurrentLevel, CurrentScenario, Delete, Group,
        MobileRobotMarker, NameInSite, Scenario, ScenarioMarker, Task, Tasks,
    },
    widgets::prelude::*,
    widgets::{view_scenarios::ScenarioDisplay, Icons},
};

use mapf::negotiation::{Agent, Obstacle, Scenario as MapfScenario};
use mapf::negotiation::*;
use std::collections::{BTreeMap, HashMap};

#[derive(SystemParam)]
pub struct MapfConfigWidget<'w, 's> {
    simulation_config: ResMut<'w, SimulationConfig>,
    debug_mode: Res<'w, State<DebugMode>>,
    debug_mode_next: ResMut<'w, NextState<DebugMode>>,
    mobile_robots: Query<'w, 's, &'static Tasks<Entity>, (With<MobileRobotMarker>, Without<Group>)>,
    current_level: Res<'w, CurrentLevel>,
    grids: Query<'w, 's, (Entity, &'static Grid)>,
    parents: Query<'w, 's, &'static Parent>,
    negotiation_request: EventWriter<'w, NegotiationRequest>,
    negotiation_params: ResMut<'w, NegotiationParams>,
    negotiation_data: ResMut<'w, NegotiationData>,
}

impl<'w, 's> WidgetSystem<Tile> for MapfConfigWidget<'w, 's> {
    fn show(_: Tile, ui: &mut Ui, state: &mut SystemState<Self>, world: &mut World) -> () {
        let mut params = state.get_mut(world);
        ui.separator();

        CollapsingHeader::new("MAPF Configuration")
            .default_open(true)
            .show(ui, |ui| {
                ComboBox::from_id_source("mapf_debug_mode")
                    .selected_text(params.debug_mode.get().label())
                    .show_ui(ui, |ui| {
                        for label in DebugMode::labels() {
                            if ui
                                .selectable_label(params.debug_mode.get().label() == label, label)
                                .clicked()
                            {
                                params.debug_mode_next.set(DebugMode::from_label(label));
                            }
                        }
                    });

                match params.debug_mode.get() {
                    DebugMode::Negotiation => params.show_negotiation(ui),
                    DebugMode::Planner => params.show_planner(ui),
                }
            });
    }
}

impl<'w, 's> MapfConfigWidget<'w, 's> {
    pub fn show_negotiation(&mut self, ui: &mut Ui) {
        // Agents with Task
        let num_tasks = self
            .mobile_robots
            .iter()
            .filter(|tasks| {
                tasks.0.iter().any(|task| {
                    if let Task::GoToPlace { location: _ } = task {
                        true
                    } else {
                        false
                    }
                })
            })
            .count();
        ui.label(format!("Tasks:    {}", num_tasks));
        // Grid Info
        let occupancy_grid = self
            .grids
            .iter()
            .filter_map(|(grid_entity, grid)| {
                if let Some(level_entity) = self.current_level.0 {
                    if self
                        .parents
                        .get(grid_entity)
                        .is_ok_and(|parent_entity| parent_entity.get() == level_entity)
                    {
                        Some(grid)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .next();
        ui.label("Occupancy");
        ui.indent("occupancy_map_info", |ui| {
            if let Some(grid) = occupancy_grid {
                ui.label(format!("    cell size: {}m", grid.cell_size));
                ui.label(format!(
                    "    grid size: {} x {}",
                    grid.range.max_cell().x - grid.range.min_cell().x,
                    grid.range.max_cell().y - grid.range.min_cell().y
                ));
            } else {
                ui.label("None");
            }
        });

        // Generate Plan
        ui.horizontal(|ui| {
            let allow_generate_plan = num_tasks > 0
                && self.negotiation_params.queue_length_limit > 0
                && !self.negotiation_data.is_in_progress();

            ui.add_enabled_ui(allow_generate_plan, |ui| {
                if ui.button("Generate Plan").clicked() {
                    self.negotiation_request.send(NegotiationRequest);
                }
            });
            ui.add(
                DragValue::new(&mut self.negotiation_params.queue_length_limit)
                    .clamp_range(0..=std::usize::MAX)
                    .speed(1000),
            );
        });

        if ui.button("Negotiation Test").clicked() {
            negotiation_test();
        }

        // Results
        ui.separator();
        match self.negotiation_data.as_ref() {
            NegotiationData::Complete {
                elapsed_time,
                solution,
                negotiation_history,
                entity_id_map,
                error_message,
                conflicting_endpoints,
            } => {
                EguiGrid::new("negotiation_data")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label("execution time");
                        ui.label(format!("{:.2} s", elapsed_time.as_secs_f32()));
                        ui.end_row();
                        ui.label("negotiation history");
                        ui.label(format!("{}", negotiation_history.len()));
                        ui.end_row();
                        ui.label("endpoint conflicts");
                        ui.label(format!("{}", conflicting_endpoints.len()));
                        ui.end_row();
                        ui.label("error message");
                        ui.label(error_message.clone().unwrap_or("None".to_string()));
                    });
            }
            NegotiationData::InProgress { start_time } => {
                let elapsed_time = start_time.elapsed();
                ui.label(format!("In Progress: {}", elapsed_time.as_secs_f32()));
            }
            _ => {}
        }

        ui.label("Nodes: ");
        ui.label("Successful in : ");
    }

    pub fn show_planner(&mut self, ui: &mut Ui) {
        ui.label("Unavailable");
    }
}

pub fn negotiation_test() {
    let mut agents: BTreeMap<String, Agent> = BTreeMap::new();
    agents.insert(
        "A".to_string(),
        Agent {
            start: get_cell(5.0, 5.0, 1.0),
            goal: get_cell(1.0, 1.0, 1.0),
            yaw: 1.0,
            radius: 0.5,
            speed: 1.0,
            spin: 1.0,
        },
    );
    let obstacles: Vec<Obstacle> = Vec::new();
    let occupancy: HashMap<i64, Vec<i64>> = HashMap::new();
    let cell_size = 0.2;

    let scenario = MapfScenario {
        agents,
        obstacles,
        occupancy,
        cell_size,
        camera_bounds: None,
    };

    let res = match negotiate(&scenario, Some(1_000_000)) {
        Ok(res) => res,
        Err(err) => {
            match err {
                NegotiationError::PlanningFailed((nodes, name_map)) => {
                    println!("Unable to find a solution");
                    for node in nodes {
                        println!("{:?}", node);
                    }
                }
                err => println!("Error while planning: {err:?}"),
            };
            return;
        }
    };
}

pub fn get_cell(x: f64, y: f64, cell_size: f64) -> [i64; 2] {
    [
        (x / cell_size).floor() as i64,
        (y / cell_size).floor() as i64,
    ]
}