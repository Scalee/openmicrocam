//! The layout the camera's MI libraries and kernel modules read, as constants: every declared struct's size and alignment,
//! the offsets of its named fields, and every declared enum value. A change to `mi.h` that moves any of them fails the build.
use core::mem::{align_of, offset_of, size_of};

use crate::*;

macro_rules! layout {
    ($($t:ident $size:literal $align:literal { $($f:ident $off:literal),* })*) => {$(
        const _: () = assert!(size_of::<$t>() == $size && align_of::<$t>() == $align, stringify!($t));
        $(const _: () = assert!(offset_of!($t, $f) == $off, concat!(stringify!($t), ".", stringify!($f)));)*
    )*};
}

macro_rules! values {
    ($($e:ident { $($v:ident $n:literal),* })*) => {$($(
        const _: () = assert!($e::$v == $n, stringify!($v));
    )*)*};
}

layout! {
    MI_SYS_ChnPort_t 0x10 4 { eModId 0, u32DevId 4, u32ChnId 8, u32PortId 0xc }
    MI_SYS_WindowRect_t 8 2 { u16X 0, u16Y 2, u16Width 4, u16Height 6 }
    MI_SYS_WindowSize_t 4 2 { u16Width 0, u16Height 2 }
    MI_SYS_FrameData_t 0x68 8 { u16Width 0x18, u16Height 0x1a, phyAddr 0x28, u32Stride 0x40, u32BufSize 0x4c }
    MI_SYS_BufInfo_t 0x108 8 { u64Pts 0, __bindgen_anon_1 0x20 }
    MI_SYS_BufInfo_t__bindgen_ty_1 0xe8 8 { stFrameData 0 }
    MI_SYS_BufFrameConfig_t 0x10 4 { u16Width 0, u16Height 2, eFrameScanMode 4, eFormat 8 }
    MI_SYS_BufConf_t 0x78 8 { eBufType 0, u64TargetPts 8, __bindgen_anon_1 0x18 }
    MI_SYS_BufConf_t__bindgen_ty_1 0x60 8 { stFrameCfg 0 }
    MI_SYS_Version_t 0x80 1 { u8Version 0 }
    MI_SNR_Res_t 0x34 4 { stOutputSize 8, u32MaxFps 0xc, strResDesc 0x14 }
    MI_SNR_PADInfo_t 0x34 4 {}
    MI_SNR_PlaneInfo_t 0x48 4 { stCapRect 0x24, eBayerId 0x2c, ePixPrecision 0x30, ePixel 0x44 }
    MI_VIF_GroupAttr_t 0x1c 4 { eIntfMode 0, eWorkMode 4, eHDRType 8, eClkEdge 0xc }
    MI_VIF_DevAttr_t 0x14 4 { eInputPixel 0, stInputRect 4, eField 0xc, bEnH2T1PMode 0x10 }
    MI_VIF_OutputPortAttr_t 0x14 4 { stCapRect 0, stDestSize 8, ePixFormat 0xc, eFrameRate 0x10 }
    MI_VIF_DevPortStat_t 0x1c 4 { u32IntCnt 4 }
    MI_ISP_AE_EXPO_VALUE_TYPE_t 0x10 4 { u32SensorGain 4, u32ISPGain 8, u32US 0xc }
    MI_ISP_AE_EXPO_INFO_TYPE_t 0x23c 4 { stExpoValueLong 8 }
    MI_ISP_AE_EV_COMP_TYPE_t 8 4 { s32EV 0, u32Grad 4 }
    MI_ISP_AWB_QUERY_INFO_TYPE_t 0x18 4 { u16Rgain 4, u16Grgain 6, u16Bgain 0xa, u16ColorTemp 0xc }
    MWB_ATTR_PARAM_t 8 2 { u16Rgain 0, u16Grgain 2, u16Gbgain 4, u16Bgain 6 }
    MI_ISP_AWB_ATTR_TYPE_t 0x5b8 4 { eOpType 4, stManualParaAPI 8 }
    MI_ISP_AWB_CTMWB_PARAM_t 4 4 { u32CT 0 }
    MI_ISP_IQ_COLORTOGRAY_TYPE_t 4 4 { bEnable 0 }
    LEVEL_BASE_PARAM_t 4 4 { u32Lev 0 }
    CONTRAST_MANUAL_ATTR_t 4 4 { stParaAPI 0 }
    CONTRAST_AUTO_ATTR_t 0x40 4 { stParaAPI 0 }
    MI_ISP_IQ_CONTRAST_TYPE_t 0x4c 4 { bEnable 0, stAuto 8, stManual 0x48 }
    BRIGHTNESS_MANUAL_ATTR_t 4 4 { stParaAPI 0 }
    BRIGHTNESS_AUTO_ATTR_t 0x40 4 { stParaAPI 0 }
    MI_ISP_IQ_BRIGHTNESS_TYPE_t 0x4c 4 { bEnable 0, stAuto 8, stManual 0x48 }
    RGBGAMMA_PARAM_t 0x600 2 { u16LutR 0, u16LutG 0x200, u16LutB 0x400 }
    RGBGAMMA_MANUAL_ATTR_t 0x600 2 { stParaAPI 0 }
    RGBGAMMA_AUTO_ATTR_t 0x6000 2 { stParaAPI 0 }
    MI_ISP_IQ_RGBGAMMA_TYPE_t 0x6608 4 { bEnable 0, stAuto 8, stManual 0x6008 }
    SATURATION_PARAM_t 0x18 1 { u8SatAllStr 0 }
    SATURATION_MANUAL_ATTR_t 0x18 1 { stParaAPI 0 }
    SATURATION_AUTO_ATTR_t 0x180 1 { stParaAPI 0 }
    MI_ISP_IQ_SATURATION_TYPE_t 0x1a0 4 { bEnable 0, stAuto 8, stManual 0x188 }
    SHARPNESS_PARAM_t 0x120 2 { u8SharpnessUD 0, u8SharpnessD 3 }
    SHARPNESS_MANUAL_ATTR_t 0x120 2 { stParaAPI 0 }
    SHARPNESS_AUTO_ATTR_t 0x1200 2 { stParaAPI 0 }
    MI_ISP_IQ_SHARPNESS_TYPE_t 0x1328 4 { bEnable 0, stAuto 8, stManual 0x1208 }
    MI_ISP_IQ_WDR_TYPE_t 0x668 4 { bEnable 0 }
    MI_ISP_DevAttr_t 4 4 { u32DevStitchMask 0 }
    MI_ISP_ChannelAttr_t 0x50 4 { u32SensorBindId 0 }
    MI_ISP_ChnParam_t 0x10 4 { eHDRType 0, e3DNRLevel 4, bMirror 8, bFlip 9, eRot 0xc }
    MI_ISP_OutPortParam_t 0xc 4 { stCropRect 0, ePixelFormat 8 }
    MI_SCL_DevAttr_t 4 4 { u32NeedUseHWOutPortMask 0 }
    MI_SCL_ChannelAttr_t 4 4 { u32Reserved 0 }
    MI_SCL_ChnParam_t 4 4 { eRot 0 }
    MI_SCL_OutPortParam_t 0x18 4 {
        stSCLOutCropRect 0, stSCLOutputSize 8, bMirror 0xc, bFlip 0xd, ePixelFormat 0x10, eCompressMode 0x14
    }
    MI_SCL_DirectBuf_t 0x30 8 {
        ePixelFormat 0, u32Width 4, u32Height 8, phyAddr 0x10, u32Stride 0x20, u32BuffSize 0x28
    }
    MI_VENC_Pack_t 0xb8 8 { pu8Addr 8, u32Len 0xc, u64PTS 0x10 }
    MI_VENC_StreamInfoH264_t 0x38 4 { eRefType 0x2c }
    MI_VENC_StreamInfoH265_t 0x30 4 { eRefType 0x24 }
    MI_VENC_Stream_t 0x48 4 { pstPack 0, u32PackCount 4, __bindgen_anon_1 0x10 }
    MI_VENC_Stream_t__bindgen_ty_1 0x38 4 { stH264Info 0, stH265Info 0 }
    MI_VENC_AttrH264_t 0x24 4 {
        u32MaxPicWidth 0, u32MaxPicHeight 4, u32Profile 0xc, bByFrame 0x10, u32PicWidth 0x14, u32PicHeight 0x18,
        u32BFrameNum 0x1c
    }
    MI_VENC_AttrJpeg_t 0x20 4 {
        u32MaxPicWidth 0, u32MaxPicHeight 4, u32BufSize 8, bByFrame 0xc, u32PicWidth 0x10, u32PicHeight 0x14
    }
    MI_VENC_AttrH265_t 0x24 4 {
        u32MaxPicWidth 0, u32MaxPicHeight 4, u32Profile 0xc, bByFrame 0x10, u32PicWidth 0x14, u32PicHeight 0x18,
        u32BFrameNum 0x1c
    }
    MI_VENC_Attr_t 0x28 4 { eType 0, __bindgen_anon_1 4 }
    MI_VENC_Attr_t__bindgen_ty_1 0x24 4 { stAttrH264e 0, stAttrJpeg 0, stAttrH265e 0 }
    MI_VENC_ChnStat_t 0x28 4 { u32LeftStreamFrames 8, u32CurPacks 0x10 }
    MI_VENC_ParamJpeg_t 0x88 4 { u32Qfactor 0, au8YQt 4, au8CbCrQt 0x44 }
    MI_VENC_AttrH264Cbr_t 0x18 4 {
        u32Gop 0, u32StatTime 4, u32SrcFrmRateNum 8, u32SrcFrmRateDen 0xc, u32BitRate 0x10, u32FluctuateLevel 0x14
    }
    MI_VENC_AttrMjpegFixQp_t 0xc 4 { u32SrcFrmRateNum 0, u32SrcFrmRateDen 4, u32Qfactor 8 }
    MI_VENC_AttrH265Cbr_t 0x18 4 {
        u32Gop 0, u32StatTime 4, u32SrcFrmRateNum 8, u32SrcFrmRateDen 0xc, u32BitRate 0x10, u32FluctuateLevel 0x14
    }
    MI_VENC_RcAttr_t 0x24 4 { eRcMode 0, __bindgen_anon_1 4 }
    MI_VENC_RcAttr_t__bindgen_ty_1 0x1c 4 { stAttrH264Cbr 0, stAttrMjpegFixQp 0, stAttrH265Cbr 0 }
    MI_VENC_ChnAttr_t 0x4c 4 { stVeAttr 0, stRcAttr 0x28 }
    MI_VENC_InputSourceConfig_t 4 4 { eInputSrcBufferMode 0 }
    MI_DISP_PubAttr_t 0x34 4 { u32BgColor 0, eIntfType 4, eIntfSync 8 }
    MI_DISP_VidWinRect_t 8 2 { u16X 0, u16Y 2, u16Width 4, u16Height 6 }
    MI_DISP_VideoLayerSize_t 8 4 { u16Width 0, u16Height 4 }
    MI_DISP_VideoLayerAttr_t 0x14 4 { stVidLayerDispWin 0, stVidLayerSize 8, ePixFormat 0x10 }
    MI_DISP_InputPortAttr_t 0xc 2 { stDispWin 0, u16SrcWidth 8, u16SrcHeight 0xa }
    MI_HDMI_InitParam_t 8 4 { pfnHdmiEventCallback 0, pCallBackArgs 4 }
    MI_HDMI_VideoAttr_t 0x18 4 { bEnableVideo 0, eTimingType 4, eOutputMode 8, eColorType 0xc, eDeepColorMode 0x14 }
    MI_HDMI_AudioAttr_t 0x10 4 { bEnableAudio 0 }
    MI_HDMI_Attr_t 0x30 4 { bConnect 0, stVideoAttr 4, stAudioAttr 0x1c }
    MI_HDMI_Edid_t 0x208 4 { bEdidValid 0, u32Edidlength 4, au8Edid 8 }
    MI_RGN_Attr_t 0x10 4 { eType 0, ePixelFmt 4, u32Width 8, u32Height 0xc }
    MI_RGN_ChnPort_t 0x10 4 { eModId 0, s32DevId 4, s32ChnId 8, s32PortId 0xc }
    MI_RGN_Point_t 8 4 { u32X 0, u32Y 4 }
    MI_RGN_Size_t 8 4 { u32Width 0, u32Height 4 }
    MI_RGN_OsdChnPortParam_t 0x1c 4 { u32Layer 0, eAlphaMode 4, u8BgAlpha 8, u8FgAlpha 9 }
    MI_RGN_CoverChnPortParam_t 0x10 4 { u32Layer 0, stSize 4, u32Color 0xc }
    MI_RGN_ChnPortParam_t 0x28 4 { bShow 0, stPoint 4, unPara 0xc }
    MI_RGN_ChnPortParam_t__bindgen_ty_1 0x1c 4 { stOsdChnPort 0, stCoverChnPort 0 }
    MI_RGN_CanvasInfo_t 0x20 8 { phyAddr 0, u32Stride 0x14 }
    MI_RGN_PaletteTable_t 0x400 1 {}
    MI_VDEC_InitParam_t 4 4 { u32Reserved 0 }
    MI_VDEC_VideoAttr_t 8 4 { u32RefFrameNum 0 }
    MI_VDEC_ChnAttr_t 0x24 4 {
        eCodecType 0, u32BufSize 4, u32PicWidth 0xc, u32PicHeight 0x10, eVideoMode 0x14, eDpbBufMode 0x18,
        stVdecVideoAttr 0x1c
    }
    MI_VDEC_VideoStream_t 0x18 8 { pu8Addr 0, u32Len 4, u64PTS 8, bEndOfFrame 0x10, bEndOfStream 0x11 }
    MI_VDEC_ChnStat_t 0x20 4 { u32LeftStreamFrames 8, u32RecvStreamFrames 0x10, u32DecodeStreamFrames 0x14 }
    MI_VDEC_OutputPortAttr_t 4 2 { u16Width 0, u16Height 2 }
}

values! {
    MI_ModuleId_e {
        E_MI_MODULE_ID_VENC 2, E_MI_MODULE_ID_VIF 6, E_MI_MODULE_ID_VDEC 8, E_MI_MODULE_ID_DISP 15,
        E_MI_MODULE_ID_ISP 33, E_MI_MODULE_ID_SCL 34
    }
    MI_SYS_BayerId_e { E_MI_SYS_PIXEL_BAYERID_MAX 12 }
    MI_SYS_PixelFormat_e {
        E_MI_SYS_PIXEL_FRAME_YUV422_YUYV 0, E_MI_SYS_PIXEL_FRAME_ARGB8888 1, E_MI_SYS_PIXEL_FRAME_ABGR8888 2,
        E_MI_SYS_PIXEL_FRAME_YUV_SEMIPLANAR_420 11, E_MI_SYS_PIXEL_FRAME_RGB_BAYER_BASE 20
    }
    MI_SYS_CompressMode_e { E_MI_SYS_COMPRESS_MODE_NONE 0 }
    MI_SYS_FrameScanMode_e { E_MI_SYS_FRAME_SCAN_MODE_PROGRESSIVE 0 }
    MI_SYS_FieldType_e { E_MI_SYS_FIELDTYPE_NONE 0 }
    MI_SYS_BufDataType_e { E_MI_SYS_BUFDATA_FRAME 1 }
    MI_SYS_Rotate_e { E_MI_SYS_ROTATE_NONE 0 }
    MI_SYS_BindType_e { E_MI_SYS_BIND_TYPE_FRAME_BASE 1, E_MI_SYS_BIND_TYPE_REALTIME 4, E_MI_SYS_BIND_TYPE_HW_RING 16 }
    MI_VIF_IntfMode_e { E_MI_VIF_MODE_MIPI 4 }
    MI_VIF_WorkMode_e { E_MI_VIF_WORK_MODE_1MULTIPLEX 0 }
    MI_VIF_FrameRate_e { E_MI_VIF_FRAMERATE_FULL 0 }
    MI_VIF_ClkEdge_e { E_MI_VIF_CLK_EDGE_DOUBLE 2 }
    MI_VIF_HDRType_e { E_MI_VIF_HDR_TYPE_OFF 0 }
    MI_ISP_AE_MODE_TYPE_e { SS_AE_MODE_A 0, SS_AE_MODE_M 4 }
    MI_ISP_AE_FLICKER_TYPE_e { SS_AE_FLICKER_TYPE_DISABLE 0, SS_AE_FLICKER_TYPE_60HZ 1, SS_AE_FLICKER_TYPE_50HZ 2 }
    MI_ISP_AWB_MODE_TYPE_e { SS_AWB_MODE_AUTO 0, SS_AWB_MODE_MANUAL 1, SS_AWB_MODE_CTMANUAL 2 }
    MI_ISP_HDRType_e { E_MI_ISP_HDR_TYPE_OFF 0 }
    MI_ISP_Rotate_e { E_MI_ISP_ROTATE_NONE 0 }
    MI_SCL_FilterType_e { E_MI_SCL_FILTER_TYPE_AUTO 0 }
    MI_VENC_ModType_e { E_MI_VENC_MODTYPE_H264E 2, E_MI_VENC_MODTYPE_H265E 3, E_MI_VENC_MODTYPE_JPEGE 4 }
    MI_VENC_RcMode_e { E_MI_VENC_RC_MODE_H264CBR 1, E_MI_VENC_RC_MODE_MJPEGFIXQP 7, E_MI_VENC_RC_MODE_H265CBR 8 }
    MI_VENC_InputSrcBufferMode_e { E_MI_VENC_INPUT_MODE_NORMAL_FRMBASE 0, E_MI_VENC_INPUT_MODE_RING_ONE_FRM 1 }
    MI_DISP_Interface_e { E_MI_DISP_INTF_HDMI 5 }
    MI_DISP_OutputTiming_e {
        E_MI_DISP_OUTPUT_480P60 6, E_MI_DISP_OUTPUT_576P50 7, E_MI_DISP_OUTPUT_720P50 8, E_MI_DISP_OUTPUT_720P60 9,
        E_MI_DISP_OUTPUT_1080P24 10, E_MI_DISP_OUTPUT_1080P25 11, E_MI_DISP_OUTPUT_1080P30 12,
        E_MI_DISP_OUTPUT_1080P50 15, E_MI_DISP_OUTPUT_1080P60 16, E_MI_DISP_OUTPUT_2560x1440_30 28,
        E_MI_DISP_OUTPUT_2560x1440_50 29, E_MI_DISP_OUTPUT_2560x1440_60 30, E_MI_DISP_OUTPUT_3840x2160_25 32,
        E_MI_DISP_OUTPUT_3840x2160_30 33
    }
    MI_DISP_SyncMode_e { E_MI_DISP_SYNC_MODE_FREE_RUN 2 }
    MI_HDMI_DeviceId_e { E_MI_HDMI_ID_0 0 }
    MI_HDMI_EventType_e { E_MI_HDMI_EVENT_HOTPLUG 0, E_MI_HDMI_EVENT_NO_PLUG 1 }
    MI_HDMI_TimingType_e {
        E_MI_HDMI_TIMING_480_60P 1, E_MI_HDMI_TIMING_576_50P 3, E_MI_HDMI_TIMING_720_50P 4, E_MI_HDMI_TIMING_720_60P 5,
        E_MI_HDMI_TIMING_1080_50P 7, E_MI_HDMI_TIMING_1080_60P 9, E_MI_HDMI_TIMING_1080_30P 10,
        E_MI_HDMI_TIMING_1080_25P 11, E_MI_HDMI_TIMING_1080_24P 12, E_MI_HDMI_TIMING_4K2K_30P 13,
        E_MI_HDMI_TIMING_1440_50P 14, E_MI_HDMI_TIMING_1440_60P 15, E_MI_HDMI_TIMING_1440_30P 17,
        E_MI_HDMI_TIMING_4K2K_25P 24
    }
    MI_HDMI_ColorType_e { E_MI_HDMI_COLOR_TYPE_YCBCR444 2 }
    MI_HDMI_OutputMode_e { E_MI_HDMI_OUTPUT_MODE_HDMI 0 }
    MI_HDMI_DeepColor_e { E_MI_HDMI_DEEP_COLOR_24BIT 0 }
    MI_RGN_Type_e { E_MI_RGN_TYPE_OSD 0, E_MI_RGN_TYPE_COVER 1 }
    MI_RGN_PixelFormat_e { E_MI_RGN_PIXEL_FORMAT_ARGB4444 1 }
    MI_RGN_AlphaMode_e { E_MI_RGN_PIXEL_ALPHA 0, E_MI_RGN_CONSTANT_ALPHA 1 }
    MI_VDEC_CodecType_e { E_MI_VDEC_CODEC_TYPE_H264 0, E_MI_VDEC_CODEC_TYPE_H265 1 }
    MI_VDEC_VideoMode_e { E_MI_VDEC_VIDEO_MODE_FRAME 1 }
    MI_VDEC_DpbBufMode_e { E_MI_VDEC_DPB_MODE_NORMAL 0 }
}
