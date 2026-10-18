//! Bind the manager to the current board materials and authored world queries.
use super::{
    grind_materials::GrindMaterials,
    player_input::grind::{self, Host, MaterialMode},
    settings::PhysicsSettings,
};
use skate_core::{
    air::trajectory::grind_surface::{Probe, ProbeHit},
    physics::{board_runtime::BoardRuntime, board_world::BoardWorld, grind_contact::balance},
};

pub(super) struct LiveHost<'a> {
    pub board: &'a mut BoardRuntime,
    pub settings: &'a mut PhysicsSettings,
    /// The dynamic props' collision layer: retail's grind line queries see DMO collision too (one world), so a
    /// grind on a prop rail finds the prop's surface. Nearest hit of both worlds; map first on a tie.
    pub props: Option<&'a BoardWorld>,
    pub materials: &'a GrindMaterials,
}

impl Host for LiveHost<'_> {
    fn apply_material_mode(&mut self, mode: MaterialMode) -> Result<(), String> {
        self.materials.apply(mode, self.board, self.settings);
        Ok(())
    }
    fn surface_probe(
        &mut self,
        world: &BoardWorld,
        actor: [u32; 2],
        index: usize,
        probe: Probe,
    ) -> Result<Option<ProbeHit>, String> {
        grind::world::surface_probe_with_props(world, self.props, actor, index, probe)
    }
    fn force_exit_line(
        &mut self,
        world: &BoardWorld,
        actor: [u32; 2],
        probe: balance::ForceExitProbe,
    ) -> Result<Option<balance::ForceExitHit>, String> {
        let map = grind::world::force_exit_line_hit(world, actor, probe)?;
        let Some(props) = self.props else { return Ok(map.map(|(_, hit)| hit)) };
        let prop = grind::world::force_exit_line_hit(props, actor, probe)?;
        Ok(match (map, prop) {
            (Some(m), Some(p)) => Some(if p.0 < m.0 { p.1 } else { m.1 }),
            (m, p) => m.or(p).map(|(_, hit)| hit),
        })
    }
}
