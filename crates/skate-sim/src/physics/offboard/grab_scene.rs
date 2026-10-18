//! Stateless offboard grab-scene execution over explicitly authored objects.
//! Ground owns pending/completion, validation and selection; this owns no ticks.
mod collision;
pub(crate) use native::{Descriptor, Hit, Line, Object, Query, Record};
use skate_core::{physics::board_world::BoardWorld, player::offboard::grab_scene as native};

///Explicit collision mesh -> physical assembly relation, not identity guessing.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MeshAssembly {
    pub mesh: u32,
    pub assembly: u32,
}

pub(crate) struct Registry {
    pub objects: native::Registry,
    mesh_assemblies: Vec<MeshAssembly>,
}
impl Registry {
    pub(crate) fn new(
        world: &BoardWorld,
        objects: Vec<Object>,
        mesh_assemblies: Vec<MeshAssembly>,
    ) -> Result<Self, String> {
        let metadata = world.query_metadata().map_err(str::to_owned)?;
        let objects = native::Registry::new(objects).map_err(str::to_owned)?;
        for (index, binding) in mesh_assemblies.iter().enumerate() {
            if binding.assembly == 0 || !metadata.meshes.iter().any(|m| m.geometry == binding.mesh)
            {
                return Err("Grab collision binding must name an existing mesh and nonzero physical assembly".into());
            }
            if mesh_assemblies[..index]
                .iter()
                .any(|prior| prior.mesh == binding.mesh)
            {
                return Err("One canonical collision mesh cannot name two assemblies".into());
            }
        }
        Ok(Self {
            objects,
            mesh_assemblies,
        })
    }
    /// Replace the traffic cars (`Provider::Vehicle`, the scene's `+4088` provider) with this tick's; the other
    /// objects stay. On a validation error the registry is unchanged.
    pub(crate) fn set_cars(&mut self, cars: Vec<Object>) -> Result<(), String> {
        let mut objects: Vec<Object> = self.objects.objects.iter().filter(|o| !matches!(o.provider, native::Provider::Vehicle)).cloned().collect();
        objects.extend(cars);
        self.objects = native::Registry::new(objects).map_err(str::to_owned)?;
        Ok(())
    }
    /// Replace the props (`Provider::LivingWorld`, the scene's `+4084` world-object provider) with this tick's.
    pub(crate) fn set_props(&mut self, props: Vec<Object>) -> Result<(), String> {
        let mut objects: Vec<Object> = self.objects.objects.iter().filter(|o| !matches!(o.provider, native::Provider::LivingWorld)).cloned().collect();
        objects.extend(props);
        self.objects = native::Registry::new(objects).map_err(str::to_owned)?;
        Ok(())
    }
    pub(crate) fn has_props(&self) -> bool {
        self.objects.objects.iter().any(|o| matches!(o.provider, native::Provider::LivingWorld))
    }
    fn assembly(&self, mesh: u32) -> Option<u32> {
        self.mesh_assemblies
            .iter()
            .find(|b| b.mesh == mesh)
            .map(|b| b.assembly)
    }
}
pub(crate) struct Scene<'a> {
    world: &'a BoardWorld,
    registry: &'a Registry,
}
impl<'a> Scene<'a> {
    pub(crate) fn new(world: &'a BoardWorld, registry: &'a Registry) -> Self {
        Self { world, registry }
    }
    pub(crate) fn query(&self, query: &Query) -> Result<Vec<Record>, String> {
        native::query(&self.registry.objects, query).map_err(str::to_owned)
    }
    ///8275F8D8 dispatches ONLY descriptor kinds1/2. Resolve the actual authored
    ///spline geometry and CURRENT physical transform, not an old query result.
    pub(crate) fn resolve(&self, descriptor: Descriptor) -> Result<Option<Record>, String> {
        if !matches!(descriptor.kind, 1 | 2) {
            return Ok(None);
        }
        for object in &self.registry.objects.objects {
            if let Some(spline) = object.splines.iter().find(|s| s.descriptor == descriptor) {
                return object.record(spline).map(Some).map_err(str::to_owned);
            }
        }
        Ok(None)
    }
    pub(crate) fn line(&self, line: Line) -> Result<Option<Hit>, String> {
        collision::line(self, line)
    }
    pub(crate) fn eligible_object(&self, hit: Hit) -> Option<u32> {
        self.registry.objects.eligible_object(hit)
    }
}
