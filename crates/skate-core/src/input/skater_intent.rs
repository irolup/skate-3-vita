//! The skater intent enum (`Sk8::eSkaterIntent`): `SkaterIntentTuple` entries of the `scoring_trick` records name
//! their intent by this index. Order from the executable's name table (TU3 0x82FCB260, 146 string pointers),
//! evidence only; the names are the identifiers the ActionGraph reads.

pub const SKATER_INTENTS: [&str; 146] = [
    "INVALID", "MarkGonzales", "DannyWay", "QuickOllie", "ForceBrake", "WipeOutRecover", "WipeOutRequest",
    "WipeOutPushOff", "GestureDownStart", "GestureLeftStart", "GestureRightStart", "GestureUpStart",
    "GestureDownHeld", "GestureLeftHeld", "GestureRightHeld", "GestureUpHeld", "RightPush", "LeftPush", "NewPush",
    "Pushing", "BodySpin", "LeftGroundGrab", "RightGroundGrab", "NewLeftGroundGrab", "NewRightGroundGrab",
    "LeftAirGrab", "RightAirGrab", "NewLeftAirGrab", "NewRightAirGrab", "NewDismount", "Dismount", "TweakX",
    "TweakY", "BoardAdjustAngle", "BoardAdjustMag", "AnticAngle", "AnticMag", "KickTurn", "Turn", "HardTurn",
    "GrindBalanceX", "Brake", "WipeoutGestureX", "WipeoutGestureY", "WipeoutControlX", "WipeoutControlY", "Slide",
    "LeftSlideStart", "RightSlideStart", "LeftSlide", "RightSlide", "Manual", "ManualBrake", "HardTurnCrouch",
    "Crouch", "HoldPattern", "GestureSpeed", "FrontFlip", "BackFlip", "NoInput", "Trick", "TrickHeight",
    "DontMirrorTrick", "SlideFs180", "SlideBs180", "AutoPumpAngle", "AutoPumpMag", "ExitGrind", "GrabWorld",
    "PhysGrindTranslation", "PhysGrindStabilityNudge", "PhysGrindUpDown", "PhysBodySpin", "HandPlantTweakX",
    "HandPlantTweakY", "HandPlantDismount", "HandPlantOneFootRight", "HandPlantOneFootLeft", "L_F_Kickflip",
    "L_B_Kickflip", "L_F_Heelflip", "L_B_Heelflip", "L_FS_Shuvit", "L_BS_Shuvit", "FingerFlip", "FS_Varial",
    "BS_Varial", "FingerFlipShuv", "FingerFlipVar", "Ollie", "U_Nollie", "U_L_F_Kickflip", "U_L_F_Heelflip",
    "U_L_B_Kickflip", "U_L_B_Heelflip", "U_Fingerflip", "U_Kickflip", "U_Heelflip", "U_N_Kickflip", "U_N_Heelflip",
    "DarkCatch", "NewDarkCatch", "Kickflip", "Heelflip", "N_Kickflip", "N_Heelflip", "Hardflip", "InwardHeelflip",
    "VarialKickflip", "VarialHeelflip", "PopShuvit", "FSPopShuvit", "FS360PopShuvit", "ToggleOffBoardState",
    "NewToggleOffBoardState", "AI_ForceOffBoard", "OB_BipedWorldZ", "OB_BipedWorldX", "OB_BipedStickMag", "OB_Mag",
    "OB_Steer", "OB_MagHeld", "OB_Jump", "OB_Sprint", "OB_MOMag", "OB_MOSteer", "OB_ObjectMvZ", "OB_ObjectMvX",
    "OB_ObjectMvRot", "OB_LookAtX", "OB_LookAtY", "OB_HoldBoard", "OB_DropBoard", "OB_ThrowBoard",
    "OB_RetrieveBoard", "OB_WalkForward", "OB_WalkForwardRight", "OB_WalkRight", "OB_WalkBackwardRight",
    "OB_WalkBackward", "OB_WalkBackwardLeft", "OB_WalkLeft", "OB_WalkForwardLeft", "OB_DoAirBodyTweak",
    "OB_AirBodyTweakX", "OB_AirBodyTweakY",
];

/// The intent name of an enum index (`None` outside the table or for 0, `INVALID`).
pub fn name(intent: u32) -> Option<&'static str> {
    SKATER_INTENTS.get(intent as usize).copied().filter(|_| intent != 0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_trick_schedule_intents_sit_at_their_retail_indices() {
        for (i, n) in [(21, "LeftGroundGrab"), (22, "RightGroundGrab"), (33, "BoardAdjustAngle"), (34, "BoardAdjustMag"), (71, "PhysGrindUpDown")] {
            assert_eq!(super::name(i), Some(n));
        }
        assert_eq!(super::name(0), None);
        assert_eq!(super::name(146), None);
    }
}
