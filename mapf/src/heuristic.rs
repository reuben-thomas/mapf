/*
 * Copyright (C) 2022 Open Source Robotics Foundation
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

use crate::node::Cost;

pub trait Heuristic: std::fmt::Debug {
    type Error: std::fmt::Debug;
    type State;
    type Goal;
    type Cost: Cost;

    fn estimate_cost(
        &self,
        from_state: &Self::State,
        to_goal: &Self::Goal
    ) -> Result<Option<Self::Cost>, Self::Error>;
}
