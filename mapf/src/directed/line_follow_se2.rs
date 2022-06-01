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

use super::simple::Graph;
use crate::expander;
use crate::motion::{
    self, Extrapolator,
    trajectory::{Trajectory, CostCalculator},
    se2::{
        Position, Point, Rotation,
        timed_position::{
            DifferentialDriveLineFollow,
            Waypoint
        },
    },
};
use crate::node::{Cost as NodeCost, PartialKeyed, PartialKeyedClosedSet};
use crate::tree::Garden;
use std::{
    hash::Hash,
    sync::Arc,
};
use time_point::TimePoint;
use num::Zero;
use derivative::Derivative;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Side {
    Beginning,
    Finish
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Key {
    from_vertex: usize,
    to_vertex: usize,
    side: Side,
}

#[derive(Clone, Debug)]
pub struct Node<Cost: NodeCost> {
    cost: Cost,
    remaining_cost_estimate: Cost,
    total_cost_estimate: Cost,
    pub state: Waypoint,
    pub key: Option<Key>,
    pub vertex: usize,
    pub motion_from_parent: Option<Trajectory<Waypoint>>,
    pub parent: Option<Arc<Node<Cost>>>,
    pub is_start: Option<Start>,
}

impl<Cost: NodeCost> PartialKeyed for Node<Cost> {
    type Key = Key;

    fn key(&self) -> Option<Self::Key> {
        self.key
    }
}

impl<Cost: NodeCost> crate::Node for Node<Cost> {
    type Cost = Cost;
    type ClosedSet = PartialKeyedClosedSet<Self>;

    fn cost(&self) -> Self::Cost {
        return self.cost;
    }

    fn parent(&self) -> &Option<Arc<Self>> {
        return &self.parent;
    }
}

impl<Cost: NodeCost> crate::node::Informed for Node<Cost> {
    fn remaining_cost_estimate(&self) -> Self::Cost {
        return self.remaining_cost_estimate;
    }

    fn total_cost_estimate(&self) -> Self::Cost {
        return self.total_cost_estimate;
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Start {
    pub vertex: usize,
    pub orientation: Rotation,
    pub offset_location: Option<Point>,
}

impl Start {
    /// Convert the start value into a waypoint. If the start value has an
    /// invalid vertex, this will return None.
    fn to_waypoint(&self, graph: &Graph<Point>) -> Option<Waypoint> {
        if let Some(location) = self.offset_location {
            return Some(Waypoint{
                time: TimePoint::zero(),
                position: Position::new(location.coords, self.orientation.angle()),
            });
        }

        if let Some(location) = graph.vertices.get(self.vertex) {
            return Some(Waypoint{
                time: TimePoint::zero(),
                position: Position::new(location.coords, self.orientation.angle()),
            });
        }

        None
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct OrientationGoal {
    pub target: Rotation,
    pub threshold: f64,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Goal {
    pub vertex: usize,
    pub orientation: Option<OrientationGoal>,
}

impl<Cost: NodeCost> crate::expander::Goal<Node<Cost>> for Goal {
    fn is_satisfied(&self, node: &Node<Cost>) -> bool {
        if self.vertex != node.vertex {
            return false;
        }

        match self.orientation {
            Some(OrientationGoal{target, threshold}) => {
                let delta_yaw = (node.state.position.rotation / target).angle().abs();
                return delta_yaw <= threshold;
            },
            None => true
        }
    }
}

pub trait Heuristic<Cost: NodeCost> {
    fn estimate_cost(&self, from_vertex: usize, to_goal: Option<usize>) -> Option<Cost>;
}

pub trait Policy {
    type Cost: crate::Cost;
    type CostCalculator: CostCalculator<Waypoint, Cost=Self::Cost>;
    type Heuristic: Heuristic<Self::Cost>;
}

type NodeType<P> = Node<<P as Policy>::Cost>;

pub struct Expander<P: Policy> {
    graph: Arc<Graph<Point>>,
    extrapolator: Arc<DifferentialDriveLineFollow>,
    cost_calculator: Arc<P::CostCalculator>,
    heuristic: P::Heuristic,
}

pub struct InitialNodes<'a, P: Policy> {
    start: &'a Start,
    expanded: bool,
    expander: &'a Expander<P>,
    goal: Option<&'a Goal>,
}

impl<'a, P: Policy> InitialNodes<'a, P> {
    fn new(expander: &'a Expander<P>, from_start: &'a Start, toward_goal: Option<&'a Goal>) -> Self {
        Self{
            start: from_start,
            expanded: false,
            expander,
            goal: toward_goal,
        }
    }
}

impl<'a, P: Policy> Iterator for InitialNodes<'a, P> {
    type Item = Result<Arc<Node<P::Cost>>, ()>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.expanded {
            return None;
        }
        self.expanded = true;

        if let Some(remaining_cost_estimate) = self.expander.heuristic
            .estimate_cost(self.start.vertex, self.goal.map(|g| g.vertex))
        {
            let waypoint = self.start.to_waypoint(self.expander.graph.as_ref())?;
            return Some(Ok(Arc::new(Node{
                cost: P::Cost::zero(),
                remaining_cost_estimate,
                total_cost_estimate: remaining_cost_estimate,
                state: waypoint,
                key: None,
                vertex: self.start.vertex,
                motion_from_parent: None,
                parent: None,
                is_start: Some(self.start.clone()),
            })));
        }

        return None;
    }
}

pub struct Expansion<'a, P: Policy> {
    from_node: Arc<NodeType<P>>,
    expansion_index: usize,
    expanded_start: bool,
    expanded_goal_orientation: bool,
    expander: &'a Expander<P>,
    goal: Option<&'a Goal>,
}

impl<'a, P: Policy> Expansion<'a, P> {
    fn new(expander: &'a Expander<P>, from_node: Arc<NodeType<P>>, toward_goal: Option<&'a Goal>) -> Self {
        Self{
            from_node,
            expansion_index: 0,
            expanded_start: false,
            expanded_goal_orientation: false,
            expander,
            goal: toward_goal,
        }
    }
}


impl<'a, P: Policy> Expansion<'a, P> {
    fn expand_to(&self, to_vertex: usize) -> Option<Arc<NodeType<P>>> {
        if let Some(vertex) = self.expander.graph.vertices.get(to_vertex) {
            let extrapolation = self.expander.extrapolator.extrapolate(
                &self.from_node.state,
                vertex
            );
            if let Ok(mut waypoints) = extrapolation {
                let state = waypoints.last().unwrap_or(&self.from_node.state).clone();
                waypoints.insert(0, self.from_node.state);
                let trajectory = Trajectory::from_iter(waypoints.into_iter()).ok();
                let remaining_cost_estimate = self.expander.heuristic.estimate_cost(to_vertex, self.goal.map(|g| g.vertex))?;
                return Some(self.expander.make_node(state, to_vertex, remaining_cost_estimate, trajectory, &self.from_node));
            }
            // else: We should propogate these inner expansion errors somehow so
            // users can be aware of unexpected problems.
        }

        return None;
    }

    fn rotate_towards_target(&self, orientation_goal: OrientationGoal) -> Option<Arc<NodeType<P>>> {
        let to_target = Position::from_parts(
            self.from_node.state.position.translation,
            orientation_goal.target
        );
        let extrapolation = self.expander.extrapolator.extrapolate(
            &self.from_node.state, &to_target
        );
        if let Ok(mut waypoints) = extrapolation {
            let state = waypoints.last().unwrap_or(&self.from_node.state).clone();
            waypoints.insert(0, self.from_node.state);
            let trajectory = Trajectory::from_iter(waypoints.into_iter()).ok();
            let remaining_cost_estimate = P::Cost::zero();
            return Some(self.expander.make_node(state, self.from_node.vertex, remaining_cost_estimate, trajectory, &self.from_node));
        }

        return None;
    }

    fn expand_from_start(&self, to_vertex: usize) -> Option<Arc<NodeType<P>>> {
        if let Some(to_target) = self.expander.graph.vertices.get(to_vertex) {
            let extrapolation = self.expander.extrapolator.extrapolate(
                &self.from_node.state,
                to_target
            );
            if let Ok(mut waypoints) = extrapolation {
                let state = waypoints.last().unwrap_or(&self.from_node.state).clone();
                waypoints.insert(0, self.from_node.state);
                let trajectory = Trajectory::from_iter(waypoints.into_iter()).ok();
                let remaining_cost_estimate = self.expander.heuristic.estimate_cost(to_vertex, self.goal.map(|g| g.vertex))?;
                return Some(self.expander.make_node(state, to_vertex, remaining_cost_estimate, trajectory, &self.from_node));
            }
        }

        return None;
    }
}

impl<'a, P: Policy> Iterator for Expansion<'a, P> {
    type Item = Result<Arc<Node<P::Cost>>, ()>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(start) = &self.from_node.is_start {
            if let Some(_) = &start.offset_location {
                if self.expanded_start {
                    return None;
                }

                self.expanded_start = true;
                return self.expand_from_start(self.from_node.vertex).map(|n| Ok(n));
            }
        }

        loop {
            if let Some(to_vertices) = self.expander.graph.edges.get(self.from_node.vertex) {
                if let Some(to_vertex) = to_vertices.get(self.expansion_index) {
                    let next_opt = self.expand_to(*to_vertex);
                    self.expansion_index += 1;
                    if let Some(next) = next_opt {
                        return Some(Ok(next));
                    } else {
                        // If it was not possible to expand to this
                        // vertex for some reason, then try the next one.
                        continue;
                    }
                }
            }

            break;
        }

        if let Some(goal) = self.goal {
            if self.from_node.vertex == goal.vertex {
                if !self.expanded_goal_orientation {
                    self.expanded_goal_orientation = true;
                    if let Some(orientation_goal) = goal.orientation {
                        if let Some(next) = self.rotate_towards_target(orientation_goal) {
                            // We should only return here if the expansion
                            // succeeded. If it came back None then we should
                            // fall through to any other expansion that might be
                            // performable.
                            return Some(Ok(next));
                        }
                    }
                }
            }
        }

        return None;
    }
}

#[derive(Derivative)]
#[derivative(Debug, Clone)]
pub struct Solution<P: Policy> {
    cost: P::Cost,
    motion: Option<motion::se2::LinearTrajectory>,
}

impl<P: Policy> Solution<P> {
    pub fn motion(&self) -> &Option<motion::se2::LinearTrajectory> {
        &self.motion
    }
}

impl<P: Policy> expander::Solution<P::Cost> for Solution<P> {
    fn cost(&self) -> P::Cost {
        self.cost
    }
}

impl<P: Policy> crate::Expander for Expander<P> {
    type Node = NodeType<P>;
    type Start = Start;
    type Goal = Goal;
    /// The Solution will appear as None if no trajectory is actually needed for
    /// the goal to be reached (i.e. the agent is starting on its goal).
    type Solution = Solution<P>;
    type InitialNodes<'a> where P: 'a = InitialNodes<'a, P>;
    type Expansion<'a> where P: 'a = Expansion<'a, P>;
    type Error = ();

    fn start<'a>(&'a self, start: &'a Start, goal: Option<&'a Self::Goal>) -> InitialNodes<'a, P> {
        InitialNodes::new(self, start, goal)
    }

    fn expand<'a>(&'a self, parent: &Arc<Self::Node>, goal: Option<&'a Self::Goal>) -> Expansion<'a, P> {
        Expansion::new(self, parent.clone(), goal)
    }

    fn make_solution(&self, solution_node: &Arc<Self::Node>) -> Result<Self::Solution, ()> {
        let mut node: Arc<Self::Node> = solution_node.clone();
        let mut waypoints: Vec<Waypoint> = Vec::new();
        loop {
            if let Some(next_waypoints) = &node.motion_from_parent {
                for wp in next_waypoints.iter().rev() {
                    waypoints.push(*wp.clone());
                }
            }

            match &node.parent {
                Some(parent) => { node = parent.clone(); },
                None => { break; }
            }
        }

        waypoints.reverse();
        return Ok(Solution{
            cost: node.cost,
            motion: Trajectory::from_iter(waypoints.into_iter()).ok(),
        });
    }
}

impl<P: Policy> Expander<P> {
    pub fn new(
        graph: Arc<Graph<Point>>,
        extrapolator: Arc<DifferentialDriveLineFollow>,
        cost_calculator: Arc<P::CostCalculator>,
        heuristic: P::Heuristic,
    ) -> Self {
        Self{
            graph,
            extrapolator,
            cost_calculator,
            heuristic,
        }
    }

    fn make_node(
        &self,
        state: Waypoint,
        to_vertex: usize,
        remaining_cost_estimate: P::Cost,
        motion: Option<Trajectory<Waypoint>>,
        parent: &Arc<Node<P::Cost>>,
    ) -> Arc<NodeType<P>> {
        let added_cost = match &motion {
            Some(trajectory) => self.cost_calculator.compute_cost(trajectory),
            None => P::Cost::zero()
        };
        let cost = parent.cost + added_cost;
        return Arc::new(Node{
            cost,
            remaining_cost_estimate,
            total_cost_estimate: cost + remaining_cost_estimate,
            state,
            key: Some(Key{from_vertex: parent.vertex, to_vertex, side: Side::Finish}),
            vertex: to_vertex,
            motion_from_parent: motion,
            parent: Some(parent.clone()),
            is_start: None,
        });
    }
}

pub struct EuclideanHeuristic<P: Policy> {
    pub graph: Arc<Graph<Point>>,
    pub extrapolation: Arc<DifferentialDriveLineFollow>,
    pub cost_calculator: Arc<P::CostCalculator>,
}

impl<P: Policy<Cost=i64>> Heuristic<P::Cost> for EuclideanHeuristic<P> {
    fn estimate_cost(&self, from_vertex: usize, to_goal: Option<usize>) -> Option<P::Cost> {
        if let Some(to_goal) = to_goal {
            let speed = self.extrapolation.translational_speed();
            let p0 = self.graph.vertices.get(from_vertex)?;
            let p1 = self.graph.vertices.get(to_goal)?;
            let distance = (p1 - p0).norm();
            return Some(time_point::Duration::from_secs_f64(distance/speed).nanos);
        }

        return Some(P::Cost::zero());
    }
}

// pub struct ShortestPathHeuristic<P: Policy> {
//     pub garden: Garden<
// }

pub struct TimeCostCalculator;
impl CostCalculator<Waypoint> for TimeCostCalculator {
    type Cost = i64;
    fn compute_cost(&self, trajectory: &Trajectory<Waypoint>) -> i64 {
        return trajectory.duration().nanos;
    }
}

pub struct SimplePolicy;

impl Policy for SimplePolicy {
    type Cost = i64;
    type CostCalculator = TimeCostCalculator;
    type Heuristic = EuclideanHeuristic<Self>;
}

pub type SimpleExpander = Expander<SimplePolicy>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algorithm::Status;

    fn make_test_graph() -> Graph<Point> {
        /*
         * 0-----1-----2-----3
         *           /       |
         *         /         |
         *       4-----5     6
         *             |
         *             |
         *             7-----8
        */

        let mut vertices = Vec::<Point>::new();
        vertices.push(Point::new(0.0, 0.0)); // 0
        vertices.push(Point::new(1.0, 0.0)); // 1
        vertices.push(Point::new(2.0, 0.0)); // 2
        vertices.push(Point::new(3.0, 0.0)); // 3
        vertices.push(Point::new(1.0, -1.0)); // 4
        vertices.push(Point::new(2.0, -1.0)); // 5
        vertices.push(Point::new(3.0, -1.0)); // 6
        vertices.push(Point::new(2.0, -2.0)); // 7
        vertices.push(Point::new(3.0, -2.0)); // 8

        let mut edges = Vec::<Vec::<usize>>::new();
        edges.resize(9, Vec::new());
        let mut add_bidir_edge = |v0: usize, v1: usize| {
            edges.get_mut(v0).unwrap().push(v1);
            edges.get_mut(v1).unwrap().push(v0);
        };
        add_bidir_edge(0, 1);
        add_bidir_edge(1, 2);
        add_bidir_edge(2, 3);
        add_bidir_edge(2, 4);
        add_bidir_edge(3, 6);
        add_bidir_edge(4, 5);
        add_bidir_edge(5, 7);
        add_bidir_edge(7, 8);

        return Graph{vertices, edges};
    }

    fn make_test_extrapolation() -> DifferentialDriveLineFollow {
        return DifferentialDriveLineFollow::new(1.0f64, std::f64::consts::PI).unwrap();
    }

    #[test]
    fn test_simple_expander() {

        let graph = Arc::new(make_test_graph());
        let extrapolation = Arc::new(make_test_extrapolation());
        let cost_calculator = Arc::new(TimeCostCalculator);

        let expander = Arc::new(SimpleExpander::new(
            graph.clone(), extrapolation.clone(), cost_calculator.clone(),
            EuclideanHeuristic{graph, extrapolation, cost_calculator}
        ));

        let planner = crate::Planner::<SimpleExpander, crate::a_star::Algorithm>::new(expander);
        let mut progress = planner.plan(
            &Start{
                vertex: 0,
                orientation: Rotation::new(0.0),
                offset_location: None,
            },
            Goal{
                vertex: 8,
                // orientation: None
                orientation: Some(OrientationGoal{
                    target: Rotation::new(90f64.to_radians()),
                    threshold: motion::DEFAULT_ROTATIONAL_THRESHOLD
                })
            }
        ).unwrap();

        match progress.solve().unwrap() {
            Status::Solved(solution) => {
                let motion = solution.motion().as_ref().unwrap();
                println!("{:#?}", motion);
            },
            Status::Impossible => {
                assert!(false);
            },
            Status::Incomplete => {
                assert!(false);
            }
        }
    }
}
