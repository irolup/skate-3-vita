//! Explicit authored objects and physical identities. Query meshes do not
//! imply interactability, spline descriptors, or assembly identities.
use super::{Descriptor,Frame,Geometry,Hit,Record,RecordInput,Vector,AssemblyData};
use std::sync::Arc;

#[derive(Clone,Copy,Debug)]
pub enum Provider {
    ///82589130: selected record44/48, flag52 and matching assembly40.
    Dmo { selection_variant:u8,matching_group:i32,record_enabled:bool },
    ///82C4BE80: actual dynamic-object provider, no actor matching gate.
    LivingWorld,
    ///82C35B98 / 82C35840 (vtable 0x82322514, scene slot +4088, query mode bit 0x02): traffic cars. No disabled
    ///gate, no kind filter (records are type 1), assembly required (car+172 chain, b49).
    Vehicle,
}
#[derive(Clone,Debug)]
pub struct Spline { pub descriptor:Descriptor,pub geometry:Arc<Geometry>,pub word_272:u32 }
#[derive(Clone,Debug)]
pub struct Object {
    pub id:u32,
    pub provider:Provider,
    ///Real provider exclusion: Dmo record92 / LW virtual108.
    pub disabled:bool,
    pub assembly_ready:bool,
    pub assembly:Option<AssemblyData>,
    pub frame:Frame,
    pub object_vector_128:Vector,
    pub splines:Vec<Spline>,
}
impl Object {
    pub fn record(&self,spline:&Spline)->Result<Record,&'static str> {
        Record::from_geometry(RecordInput {descriptor:spline.descriptor,geometry:spline.geometry.clone(),
            frame:self.frame,object_vector_128:self.object_vector_128,assembly:self.assembly.as_ref(),word_272:spline.word_272})
    }
}
#[derive(Clone,Debug)]
pub struct Registry {pub objects:Vec<Object>}
impl Registry {
    pub fn new(objects:Vec<Object>)->Result<Self,&'static str> {
        for (index,object) in objects.iter().enumerate() {
            if object.id==0 {return Err("Interactable object ID zero is reserved for none");}
            if let Provider::Dmo {selection_variant,..}=object.provider {
                if selection_variant>1 {return Err("Dmo body selection is a single bit");}
            }
            for spline in &object.splines {
                if !matches!(spline.descriptor.kind,1|2) {return Err("Unknown native grab descriptor kind");}
                if matches!(object.provider,Provider::Vehicle)!=(spline.descriptor.kind==1) {
                    return Err("Car grab records are type 1, others type 2");
                }
                object.record(spline)?;
            }
            if objects[..index].iter().any(|prior|prior.id==object.id) {
                return Err("Duplicate authored interactable object ID");
            }
        }
        Ok(Self {objects})
    }
    pub fn eligible_object(&self,hit:Hit)->Option<u32> {
        //82760368 rejects zero distance and missing assembly BEFORE lookup.
        if hit.fraction==0.0 {return None;}
        let assembly=hit.assembly?;
        self.objects.iter().find(|object|!object.disabled && object.assembly.as_ref()
            .is_some_and(|a|a.identity==assembly)).map(|object|object.id)
    }
}
