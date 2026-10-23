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
        self.replace(|o| matches!(o.provider, native::Provider::Vehicle), cars)
    }
    /// Replace the props (`Provider::LivingWorld`, the scene's `+4084` world-object provider) with this tick's.
    pub(crate) fn set_props(&mut self, props: Vec<Object>) -> Result<(), String> {
        self.replace(|o| matches!(o.provider, native::Provider::LivingWorld), props)
    }
    /// skate_vita (performance, exact): upstream rebuilt the registry from a copy of the kept objects and the new ones
    /// (`native::Registry::new`, every object's every record checked, twice a tick). The kept objects passed those
    /// checks when they came in and are unchanged, so only the new ones are checked here, in the same order and with
    /// the same errors (`check`); on an error the registry is unchanged.
    fn replace(&mut self, replaced: impl Fn(&Object) -> bool, incoming: Vec<Object>) -> Result<(), String> {
        let previous = &self.objects.objects;
        for (k, object) in incoming.iter().enumerate() {
            let before = previous.iter().find(|p| replaced(p) && p.id == object.id);
            check(object, before).map_err(str::to_owned)?;
            if previous.iter().filter(|p| !replaced(p)).chain(&incoming[..k]).any(|prior| prior.id == object.id) {
                return Err("Duplicate authored interactable object ID".to_owned());
            }
        }
        self.objects.objects.retain(|o| !replaced(o));
        self.objects.objects.extend(incoming);
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
/// skate_vita: `native::Registry::new`'s checks of one object (verbatim but the duplicate test, made by the caller).
/// A spline the registry already held under this object id (`before`: the same descriptor, `Arc` geometry and
/// `word_272`, the same assembly identities) passed its record check then; that check reads nothing else of the object
/// but its frame and object vector, so for such a spline it fails exactly when one of them is not finite.
fn check(object: &Object, before: Option<&Object>) -> Result<(), &'static str> {
    if object.id == 0 {
        return Err("Interactable object ID zero is reserved for none");
    }
    if let native::Provider::Dmo { selection_variant, .. } = object.provider {
        if selection_variant > 1 {
            return Err("Dmo body selection is a single bit");
        }
    }
    let identities = |o: &Object| {
        o.assembly.as_ref().map(|a| (a.identity, a.first_part.as_ref().map(|p| (p.identity, p.rates.as_ref().map(|r| r.identity)))))
    };
    let same_assembly = before.is_some_and(|b| identities(b) == identities(object));
    let finite = || object.frame.iter().flatten().chain(object.object_vector_128.iter()).all(|v| v.is_finite());
    for (i, spline) in object.splines.iter().enumerate() {
        if !matches!(spline.descriptor.kind, 1 | 2) {
            return Err("Unknown native grab descriptor kind");
        }
        if matches!(object.provider, native::Provider::Vehicle) != (spline.descriptor.kind == 1) {
            return Err("Car grab records are type 1, others type 2");
        }
        let checked = same_assembly
            && before.and_then(|b| b.splines.get(i)).is_some_and(|p| {
                p.descriptor == spline.descriptor && p.word_272 == spline.word_272 && std::sync::Arc::ptr_eq(&p.geometry, &spline.geometry)
            });
        if checked {
            if !finite() {
                return Err("Nonfinite authored grab geometry/frame");
            }
        } else {
            object.record(spline)?;
        }
    }
    Ok(())
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
