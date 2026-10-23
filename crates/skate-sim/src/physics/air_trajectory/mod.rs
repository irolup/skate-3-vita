//! Real stock-settings and BoardWorld adapter for the player trajectory selector.
mod grind;
mod settings;
use crate::grind_world::StaticProvider;
pub(crate) use grind::GrindContext;
use std::sync::Arc;
mod world;
use skate_core::{
    air::trajectory::{
        LaunchInfo, QueryRequest, QueryResult, SelectorInput, SelectorSettings, TrajectorySelector,
        query_trajectory,
    },
    physics::board_world::BoardWorld,
};
use skate_data::collections::Collections;

pub struct AirTrajectoryRuntime {
    pub selector: TrajectorySelector,
    pub settings: SelectorSettings,
    pending_results: Option<Vec<QueryResult>>,
    grind_settings: grind::Settings,
    grind_world: Option<Arc<StaticProvider>>,
    /// The props' rails at the last stepped pose (`set_prop_rails` each tick; carried prop left out).
    prop_rails: crate::grind_world::props::PropRailPoses,
    car_rails: crate::grind_world::props::PropRailPoses,
    /// 82D69D68 keeps the records of the last real query (copies: props move between queries).
    nearby_grinds: Vec<crate::grind_world::RailHit>,
}
impl AirTrajectoryRuntime {
    ///Full82E099A0 query shared by trajectory and Footplant callers.
    pub fn query(world: &BoardWorld, request: QueryRequest) -> Result<QueryResult, String> {
        query_trajectory(
            request,
            |start, end, radius| world::line(world, start, end, radius),
            |position, radius| world::nearby(world, position, radius),
        )
    }
    /// A new actor on a pooled body (b299): the selector and its pending batch as a fresh body has them.
    pub(crate) fn reset_for_new_actor(&mut self) {
        self.selector = TrajectorySelector::new();
        self.pending_results = None;
        self.nearby_grinds.clear();
    }
    pub fn load(collections: &Collections) -> Result<Self, String> {
        Ok(Self {
            selector: TrajectorySelector::new(),
            settings: settings::load(collections)?,
            pending_results: None,
            grind_settings: grind::Settings::load(collections)?,
            grind_world: None,
            prop_rails: Default::default(),
            car_rails: Default::default(),
            nearby_grinds: Vec::new(),
        })
    }
    pub fn launch(
        &mut self,
        info: LaunchInfo,
        input: SelectorInput,
        world: &BoardWorld,
        props: Option<&BoardWorld>,
    ) -> Result<bool, String> {
        let launched = self.selector.launch(info, input, &self.settings)?;
        if launched {
            self.submit(world, props)?;
        }
        Ok(launched)
    }
    pub fn set_prop_rails(&mut self, props: crate::grind_world::props::PropRailPoses, cars: crate::grind_world::props::PropRailPoses) {
        self.prop_rails = props;
        self.car_rails = cars;
    }
    pub fn bind_grind_world(&mut self, provider: Arc<StaticProvider>) {
        self.grind_world = Some(provider);
        self.nearby_grinds.clear();
    }
    pub fn update(
        &mut self,
        input: SelectorInput,
        world: &BoardWorld,
        props: Option<&BoardWorld>,
        context: GrindContext,
    ) -> Result<bool, String> {
        let _trajectory = skate_core::profile::scope("z_trajectory");
        let Some(results) = self.pending_results.take() else {
            return Ok(self.selector.update_without_completion());
        };
        let provider = self
            .grind_world
            .as_deref()
            .ok_or("Trajectory static grind provider was not registered")?;
        let provider = crate::grind_world::RailView {
            rails: provider,
            props: Some(&self.prop_rails),
            cars: Some(&self.car_rails),
            skip: None,
            rules: Default::default(),
        };
        let valid = self.selector.complete_batch(
            &results,
            input,
            &self.settings,
            |prediction, acquire| {
                self.grind_settings.evaluate(
                    prediction,
                    acquire,
                    world,
                    props,
                    provider,
                    &mut self.nearby_grinds,
                    input.grind_lock_distance,
                    context,
                )
            },
            |start, end, radius| world::line_with_props(world, props, start, end, radius),
        )?;
        if self.selector.pending() {
            self.submit(world, props)?;
        }
        Ok(valid)
    }
    /// The landing prediction queries the map and the dynamic props (a jump onto a bench predicted no landing on it
    /// before 2026-10-15, so the grind aim never committed to the bench's rail).
    fn submit(&mut self, world: &BoardWorld, props: Option<&BoardWorld>) -> Result<(), String> {
        let results = self
            .selector
            .requests()
            .iter()
            .map(|&request| query_trajectory(
                request,
                |start, end, radius| world::line_with_props(world, props, start, end, radius),
                |position, radius| world::nearby_with_props(world, props, position, radius),
            ))
            .collect::<Result<Vec<_>, _>>()?;
        self.pending_results = Some(results);
        Ok(())
    }
}
