use super::*;
use std::sync::Arc;
use crate::player::offboard::grab_scene::Geometry;
fn spline(points:Vec<Vector>)->Record {
    let mut r=Record([0;72],Arc::new(Geometry {id:1,points,
        approach_vectors:vec![[0.,0.,1.,0.]],word_60:0}));
    for (i,axis) in [[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.;4]].into_iter().enumerate() {
        r.set_vector(i*16,axis);
    }
    let a=r.points()[0];let b=*r.points().last().unwrap();
    r.set_vector(64,a);r.set_vector(80,b);r.set_vector(96,[0.,0.,1.,0.]);
    r.0[49]=1;
    r.0[44]=r.points().windows(2).map(|p|length(sub(p[1],p[0]))).sum::<f32>().to_bits();
    r
}
#[test]
fn evaluates_full_polyline_not_endpoint_chord() {
    let r=spline(vec![[0.;4],[1.,0.,0.,0.],[1.,0.,1.,0.]]);
    let point=polyline::at_distance(&r,1.5);
    assert!((point[0]-1.).abs()<1e-6 && (point[2]-0.5).abs()<1e-6);
    let along=polyline::nearest_distance(&r,[1.,0.,0.5,0.]);
    assert!((along-1.5).abs()<1e-6);
}
#[test]
fn reverse_flag_changes_arc_length_traversal() {
    let mut r=spline(vec![[0.;4],[1.,0.,0.,0.],[1.,0.,1.,0.]]);
    r.0[50]|=0x20000000;
    let point=polyline::at_distance(&r,0.5);
    assert!((point[0]-1.).abs()<1e-6 && (point[2]-0.5).abs()<1e-6);
}
#[test]
fn qualification_requires_real_approach_facing_and_box() {
    let r=spline(vec![[-1.,0.,0.,0.],[1.,0.,0.,0.]]);
    let bounds=Bounds {frame:[[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,-1.,0.],[0.;4]],
        extents:[2.,1.,2.,0.]};
    let limits=BoardLimits {margin:0.1,angle_a:1.,angle_b:1.};
    assert!(qualify(&r,[0.,0.,1.,0.],bounds,limits));
    let mut away=bounds;away.frame[2]=[0.,0.,1.,0.];
    assert!(!qualify(&r,[0.,0.,1.,0.],away,limits));
    let mut remote=bounds;remote.frame[3]=[5.,0.,0.,0.];
    assert!(!qualify(&r,[0.,0.,1.,0.],remote,limits));
}
/// 82D2D550 (research b148): the skitch grab point follows a bent spline (the chord would not), counts from the other
/// end with the reverse bit (+200 0x20), and holds the ends at or past +-half (+180).
#[test]
fn along_lookup_follows_the_spline_and_its_reverse_bit() {
    let mut r=spline(vec![[0.;4],[1.,0.,0.2,0.],[2.,0.,0.,0.]]);
    r.0[45]=1.0f32.to_bits();
    let p=polyline::at_along(&r,0.0);
    assert!((p[0]-1.).abs()<1e-6 && (p[2]-0.2).abs()<1e-6,"the middle is on the bend: {p:?}");
    let p=polyline::at_along(&r,-0.5);
    assert!((p[0]-0.5).abs()<1e-6 && (p[2]-0.1).abs()<1e-6,"{p:?}");
    r.0[50]^=0x20<<24;
    let p=polyline::at_along(&r,-0.5);
    assert!((p[0]-1.5).abs()<1e-6 && (p[2]-0.1).abs()<1e-6,"the reverse bit counts from the far end: {p:?}");
    assert_eq!(polyline::at_along(&r,-1.0),r.endpoints()[0]);
    assert_eq!(polyline::at_along(&r,1.5),r.endpoints()[1]);
}
