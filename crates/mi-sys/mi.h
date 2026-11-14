/*
 * mi.h — what microcamd declares of the SigmaStar MI user libraries (libmi_*.so, libcus3a.so) of the YIZHAN 4K HDMI/USB
 * microscope camera (SigmaStar SSC8836 / Mercury6), written for this project; bindgen turns it into the `mi-sys` crate.
 *
 * Only what the daemon uses is declared. The function names and argument lists are the libraries' exported interface;
 * the struct sizes, field offsets and enum values are the layout the libraries and the kernel modules read, checked at
 * every build by `src/layout.rs`. A field the daemon does not touch has no name: its bytes stay as `_opaque_<offset>`
 * so that the size and the named fields' offsets hold. Header for the camera's ABI only (32-bit ARM, EABI enums).
 */

#ifndef MICROCAM_MI_H
#define MICROCAM_MI_H

/* ---- SYS ---- */

typedef enum {
    E_MI_MODULE_ID_VENC = 2,
    E_MI_MODULE_ID_VIF = 6,
    E_MI_MODULE_ID_VDEC = 8,
    E_MI_MODULE_ID_DISP = 15,
    E_MI_MODULE_ID_ISP = 33,
    E_MI_MODULE_ID_SCL = 34,
} MI_ModuleId_e;

typedef enum {
    E_MI_SYS_PIXEL_BAYERID_MAX = 12,
} MI_SYS_BayerId_e;

typedef enum {
    E_MI_SYS_PIXEL_FRAME_YUV422_YUYV = 0,
    E_MI_SYS_PIXEL_FRAME_ARGB8888 = 1,
    E_MI_SYS_PIXEL_FRAME_ABGR8888 = 2,
    E_MI_SYS_PIXEL_FRAME_YUV_SEMIPLANAR_420 = 11,
    E_MI_SYS_PIXEL_FRAME_RGB_BAYER_BASE = 20,
} MI_SYS_PixelFormat_e;

typedef enum {
    E_MI_SYS_COMPRESS_MODE_NONE = 0,
} MI_SYS_CompressMode_e;

typedef enum {
    E_MI_SYS_FRAME_SCAN_MODE_PROGRESSIVE = 0,
} MI_SYS_FrameScanMode_e;

typedef enum {
    E_MI_SYS_FIELDTYPE_NONE = 0,
} MI_SYS_FieldType_e;

typedef enum {
    E_MI_SYS_BUFDATA_FRAME = 1,
} MI_SYS_BufDataType_e;

typedef enum {
    E_MI_SYS_ROTATE_NONE = 0,
} MI_SYS_Rotate_e;

typedef enum {
    E_MI_SYS_BIND_TYPE_FRAME_BASE = 1,
    E_MI_SYS_BIND_TYPE_REALTIME = 4,
    E_MI_SYS_BIND_TYPE_HW_RING = 16,
} MI_SYS_BindType_e;

typedef unsigned long long MI_PHY;
typedef int MI_SYS_BUF_HANDLE;

typedef struct {
    MI_ModuleId_e eModId;
    unsigned int u32DevId;
    unsigned int u32ChnId;
    unsigned int u32PortId;
} MI_SYS_ChnPort_t;

typedef struct {
    unsigned short u16X;
    unsigned short u16Y;
    unsigned short u16Width;
    unsigned short u16Height;
} MI_SYS_WindowRect_t;

typedef struct {
    unsigned short u16Width;
    unsigned short u16Height;
} MI_SYS_WindowSize_t;

typedef struct {
    unsigned int _opaque_0[6];
    unsigned short u16Width;
    unsigned short u16Height;
    unsigned int _opaque_1c[3];
    MI_PHY phyAddr[3];
    unsigned int u32Stride[3];
    unsigned int u32BufSize;
    unsigned int _opaque_50[6];
} MI_SYS_FrameData_t;

typedef struct {
    unsigned long long u64Pts;
    unsigned int _opaque_8[6];
    union {
        MI_SYS_FrameData_t stFrameData;
        unsigned char _opaque[232];
    };
} MI_SYS_BufInfo_t;

typedef struct {
    unsigned short u16Width;
    unsigned short u16Height;
    MI_SYS_FrameScanMode_e eFrameScanMode;
    MI_SYS_PixelFormat_e eFormat;
    unsigned int _opaque_c;
} MI_SYS_BufFrameConfig_t;

typedef struct {
    MI_SYS_BufDataType_e eBufType;
    unsigned long long u64TargetPts;
    unsigned int _opaque_10[2];
    union __attribute__((aligned(8))) {
        MI_SYS_BufFrameConfig_t stFrameCfg;
        unsigned char _opaque[96];
    };
} MI_SYS_BufConf_t;

typedef struct {
    unsigned char u8Version[128];
} MI_SYS_Version_t;

int MI_SYS_Init(unsigned short u16SocId);
int MI_SYS_Exit(unsigned short u16SocId);
int MI_SYS_BindChnPort2(unsigned short u16SocId, MI_SYS_ChnPort_t *pstSrcChnPort, MI_SYS_ChnPort_t *pstDstChnPort,
                        unsigned int u32SrcFrmrate, unsigned int u32DstFrmrate, MI_SYS_BindType_e eBindType,
                        unsigned int u32BindParam);
int MI_SYS_UnBindChnPort(unsigned short u16SocId, MI_SYS_ChnPort_t *pstSrcChnPort, MI_SYS_ChnPort_t *pstDstChnPort);
int MI_SYS_GetVersion(unsigned short u16SocId, MI_SYS_Version_t *pstVersion);
int MI_SYS_GetCurPts(unsigned short u16SocId, unsigned long long *pu64Pts);
int MI_SYS_ChnInputPortGetBufPa(MI_SYS_ChnPort_t *pstChnPort, MI_SYS_BufConf_t *pstBufConf,
                                MI_SYS_BufInfo_t *pstBufInfo, MI_SYS_BUF_HANDLE *pBufHandle, int s32TimeOutMs);
int MI_SYS_ChnInputPortPutBufPa(MI_SYS_BUF_HANDLE bufHandle, MI_SYS_BufInfo_t *pstBufInfo, unsigned char bDropBuf);
int MI_SYS_ChnOutputPortGetBufPa(MI_SYS_ChnPort_t *pstChnPort, MI_SYS_BufInfo_t *pstBufInfo,
                                 MI_SYS_BUF_HANDLE *pBufHandle);
int MI_SYS_ChnOutputPortPutBufPa(MI_SYS_BUF_HANDLE bufHandle);
int MI_SYS_SetChnOutputPortDepth(unsigned short u16SocId, MI_SYS_ChnPort_t *pstChnPort, unsigned int u32UserFrameDepth,
                                 unsigned int u32BufQueueDepth);
int MI_SYS_GetFd(MI_SYS_ChnPort_t *pstChnPort, int *s32Fd);
int MI_SYS_CloseFd(int s32ChnPortFd);
int MI_SYS_MMA_Alloc(unsigned short u16SocId, unsigned char *pstMMAHeapName, unsigned int u32BlkSize, MI_PHY *phyAddr);
int MI_SYS_MMA_Free(unsigned short u16SocId, MI_PHY phyAddr);
int MI_SYS_Mmap(unsigned long long phyAddr, unsigned int u32Size, void **ppVirtualAddress, unsigned char bCache);
int MI_SYS_Munmap(void *pVirtualAddress, unsigned int u32Size);
int MI_SYS_FlushInvCache(void *pVirtualAddress, unsigned int u32Length);

/* ---- SNR ---- */

typedef struct {
    unsigned int _opaque_0[2];
    MI_SYS_WindowSize_t stOutputSize;
    unsigned int u32MaxFps;
    unsigned int _opaque_10;
    signed char strResDesc[32];
} MI_SNR_Res_t;

typedef struct {
    unsigned int _opaque_0[13];
} MI_SNR_PADInfo_t;

typedef struct {
    unsigned int _opaque_0[9];
    MI_SYS_WindowRect_t stCapRect;
    MI_SYS_BayerId_e eBayerId;
    unsigned int ePixPrecision;
    unsigned int _opaque_34[4];
    MI_SYS_PixelFormat_e ePixel;
} MI_SNR_PlaneInfo_t;

int MI_SNR_Enable(unsigned int u32PadId);
int MI_SNR_Disable(unsigned int u32PadId);
int MI_SNR_GetPadInfo(unsigned int u32PadId, MI_SNR_PADInfo_t *pstPadInfo);
int MI_SNR_GetPlaneInfo(unsigned int u32PadId, unsigned int u32PlaneID, MI_SNR_PlaneInfo_t *pstPlaneInfo);
int MI_SNR_QueryResCount(unsigned int u32PadId, unsigned int *pu32ResCount);
int MI_SNR_GetRes(unsigned int u32PadId, unsigned char u8ResIdx, MI_SNR_Res_t *pstRes);
int MI_SNR_SetRes(unsigned int u32PadId, unsigned char u8ResIdx);
int MI_SNR_SetPlaneMode(unsigned int u32PadId, unsigned char bEnable);

/* ---- VIF ---- */

typedef enum {
    E_MI_VIF_MODE_MIPI = 4,
} MI_VIF_IntfMode_e;

typedef enum {
    E_MI_VIF_WORK_MODE_1MULTIPLEX = 0,
} MI_VIF_WorkMode_e;

typedef enum {
    E_MI_VIF_FRAMERATE_FULL = 0,
} MI_VIF_FrameRate_e;

typedef enum {
    E_MI_VIF_CLK_EDGE_DOUBLE = 2,
} MI_VIF_ClkEdge_e;

typedef enum {
    E_MI_VIF_HDR_TYPE_OFF = 0,
} MI_VIF_HDRType_e;

typedef struct {
    MI_VIF_IntfMode_e eIntfMode;
    MI_VIF_WorkMode_e eWorkMode;
    MI_VIF_HDRType_e eHDRType;
    MI_VIF_ClkEdge_e eClkEdge;
    unsigned int _opaque_10[3];
} MI_VIF_GroupAttr_t;

typedef struct {
    MI_SYS_PixelFormat_e eInputPixel;
    MI_SYS_WindowRect_t stInputRect;
    MI_SYS_FieldType_e eField;
    unsigned char bEnH2T1PMode;
} MI_VIF_DevAttr_t;

typedef struct {
    MI_SYS_WindowRect_t stCapRect;
    MI_SYS_WindowSize_t stDestSize;
    MI_SYS_PixelFormat_e ePixFormat;
    MI_VIF_FrameRate_e eFrameRate;
} MI_VIF_OutputPortAttr_t;

typedef struct {
    unsigned int _opaque_0;
    unsigned int u32IntCnt;
    unsigned int _opaque_8[5];
} MI_VIF_DevPortStat_t;

int MI_VIF_CreateDevGroup(unsigned int GroupId, MI_VIF_GroupAttr_t *pstGroupAttr);
int MI_VIF_DestroyDevGroup(unsigned int GroupId);
int MI_VIF_SetDevAttr(unsigned int DevId, MI_VIF_DevAttr_t *pstDevAttr);
int MI_VIF_EnableDev(unsigned int DevId);
int MI_VIF_DisableDev(unsigned int DevId);
int MI_VIF_SetOutputPortAttr(unsigned int DevId, unsigned int PortId, MI_VIF_OutputPortAttr_t *pstAttr);
int MI_VIF_EnableOutputPort(unsigned int DevId, unsigned int PortId);
int MI_VIF_DisableOutputPort(unsigned int DevId, unsigned int PortId);
int MI_VIF_Query(unsigned int DevId, unsigned int PortId, MI_VIF_DevPortStat_t *pstStat);

/* ---- ISP picture: IQ, AE, AWB ---- */

typedef enum {
    SS_AE_MODE_A = 0,
    SS_AE_MODE_M = 4,
} MI_ISP_AE_MODE_TYPE_e;

typedef enum {
    SS_AE_FLICKER_TYPE_DISABLE = 0,
    SS_AE_FLICKER_TYPE_60HZ = 1,
    SS_AE_FLICKER_TYPE_50HZ = 2,
} MI_ISP_AE_FLICKER_TYPE_e;

typedef enum {
    SS_AWB_MODE_AUTO = 0,
    SS_AWB_MODE_MANUAL = 1,
    SS_AWB_MODE_CTMANUAL = 2,
} MI_ISP_AWB_MODE_TYPE_e;

typedef struct {
    unsigned int _opaque_0;
    unsigned int u32SensorGain;
    unsigned int u32ISPGain;
    unsigned int u32US;
} MI_ISP_AE_EXPO_VALUE_TYPE_t;

typedef struct {
    unsigned int _opaque_0[2];
    MI_ISP_AE_EXPO_VALUE_TYPE_t stExpoValueLong;
    unsigned int _opaque_18[137];
} MI_ISP_AE_EXPO_INFO_TYPE_t;

typedef struct {
    int s32EV;
    unsigned int u32Grad;
} MI_ISP_AE_EV_COMP_TYPE_t;

typedef struct {
    unsigned int _opaque_0;
    unsigned short u16Rgain;
    unsigned short u16Grgain;
    unsigned char _opaque_8[2];
    unsigned short u16Bgain;
    unsigned short u16ColorTemp;   /* the colour temperature the AWB estimates, in kelvin */
    unsigned char _opaque_e[2];
    unsigned int _opaque_10[2];
} MI_ISP_AWB_QUERY_INFO_TYPE_t;

typedef struct {
    unsigned short u16Rgain;
    unsigned short u16Grgain;
    unsigned short u16Gbgain;
    unsigned short u16Bgain;
} MWB_ATTR_PARAM_t;

typedef struct {
    unsigned int _opaque_0;
    MI_ISP_AWB_MODE_TYPE_e eOpType;
    MWB_ATTR_PARAM_t stManualParaAPI;
    unsigned int _opaque_10[362];
} MI_ISP_AWB_ATTR_TYPE_t;

typedef struct {
    unsigned int u32CT;
} MI_ISP_AWB_CTMWB_PARAM_t;

typedef struct {
    unsigned int bEnable;
} MI_ISP_IQ_COLORTOGRAY_TYPE_t;

typedef struct {
    unsigned int u32Lev;
} LEVEL_BASE_PARAM_t;

typedef struct {
    LEVEL_BASE_PARAM_t stParaAPI;
} CONTRAST_MANUAL_ATTR_t;

typedef struct {
    LEVEL_BASE_PARAM_t stParaAPI[16];
} CONTRAST_AUTO_ATTR_t;

typedef struct {
    unsigned int bEnable;
    unsigned int _opaque_4;
    CONTRAST_AUTO_ATTR_t stAuto;
    CONTRAST_MANUAL_ATTR_t stManual;
} MI_ISP_IQ_CONTRAST_TYPE_t;

typedef struct {
    LEVEL_BASE_PARAM_t stParaAPI;
} BRIGHTNESS_MANUAL_ATTR_t;

typedef struct {
    LEVEL_BASE_PARAM_t stParaAPI[16];
} BRIGHTNESS_AUTO_ATTR_t;

typedef struct {
    unsigned int bEnable;
    unsigned int _opaque_4;
    BRIGHTNESS_AUTO_ATTR_t stAuto;
    BRIGHTNESS_MANUAL_ATTR_t stManual;
} MI_ISP_IQ_BRIGHTNESS_TYPE_t;

typedef struct {
    unsigned short u16LutR[256];
    unsigned short u16LutG[256];
    unsigned short u16LutB[256];
} RGBGAMMA_PARAM_t;

typedef struct {
    RGBGAMMA_PARAM_t stParaAPI;
} RGBGAMMA_MANUAL_ATTR_t;

typedef struct {
    RGBGAMMA_PARAM_t stParaAPI[16];
} RGBGAMMA_AUTO_ATTR_t;

typedef struct {
    unsigned int bEnable;
    unsigned int _opaque_4;
    RGBGAMMA_AUTO_ATTR_t stAuto;
    RGBGAMMA_MANUAL_ATTR_t stManual;
} MI_ISP_IQ_RGBGAMMA_TYPE_t;

typedef struct {
    unsigned char u8SatAllStr;
    unsigned char _opaque_1[23];
} SATURATION_PARAM_t;

typedef struct {
    SATURATION_PARAM_t stParaAPI;
} SATURATION_MANUAL_ATTR_t;

typedef struct {
    SATURATION_PARAM_t stParaAPI[16];
} SATURATION_AUTO_ATTR_t;

typedef struct {
    unsigned int bEnable;
    unsigned int _opaque_4;
    SATURATION_AUTO_ATTR_t stAuto;
    SATURATION_MANUAL_ATTR_t stManual;
} MI_ISP_IQ_SATURATION_TYPE_t;

typedef struct {
    unsigned char u8SharpnessUD[3];
    unsigned char u8SharpnessD[3];
    unsigned short _opaque_6[141];
} SHARPNESS_PARAM_t;

typedef struct {
    SHARPNESS_PARAM_t stParaAPI;
} SHARPNESS_MANUAL_ATTR_t;

typedef struct {
    SHARPNESS_PARAM_t stParaAPI[16];
} SHARPNESS_AUTO_ATTR_t;

typedef struct {
    unsigned int bEnable;
    unsigned int _opaque_4;
    SHARPNESS_AUTO_ATTR_t stAuto;
    SHARPNESS_MANUAL_ATTR_t stManual;
} MI_ISP_IQ_SHARPNESS_TYPE_t;

typedef struct {
    unsigned int bEnable;
    unsigned int _opaque_4[409];
} MI_ISP_IQ_WDR_TYPE_t;

int MI_ISP_API_CmdLoadBinFile(unsigned int DevId, unsigned int ChnId, char *filepath, unsigned int user_key);
int MI_ISP_IQ_SetColorToGray(unsigned int DevId, unsigned int Channel, MI_ISP_IQ_COLORTOGRAY_TYPE_t *data);
int MI_ISP_IQ_SetContrast(unsigned int DevId, unsigned int Channel, MI_ISP_IQ_CONTRAST_TYPE_t *data);
int MI_ISP_IQ_GetContrast(unsigned int DevId, unsigned int Channel, MI_ISP_IQ_CONTRAST_TYPE_t *data);
int MI_ISP_IQ_SetBrightness(unsigned int DevId, unsigned int Channel, MI_ISP_IQ_BRIGHTNESS_TYPE_t *data);
int MI_ISP_IQ_GetBrightness(unsigned int DevId, unsigned int Channel, MI_ISP_IQ_BRIGHTNESS_TYPE_t *data);
int MI_ISP_IQ_SetRGBGamma(unsigned int DevId, unsigned int Channel, MI_ISP_IQ_RGBGAMMA_TYPE_t *data);
int MI_ISP_IQ_GetRGBGamma(unsigned int DevId, unsigned int Channel, MI_ISP_IQ_RGBGAMMA_TYPE_t *data);
int MI_ISP_IQ_SetSaturation(unsigned int DevId, unsigned int Channel, MI_ISP_IQ_SATURATION_TYPE_t *data);
int MI_ISP_IQ_GetSaturation(unsigned int DevId, unsigned int Channel, MI_ISP_IQ_SATURATION_TYPE_t *data);
int MI_ISP_IQ_SetSharpness(unsigned int DevId, unsigned int Channel, MI_ISP_IQ_SHARPNESS_TYPE_t *data);
int MI_ISP_IQ_GetSharpness(unsigned int DevId, unsigned int Channel, MI_ISP_IQ_SHARPNESS_TYPE_t *data);
int MI_ISP_IQ_SetWDR(unsigned int DevId, unsigned int Channel, MI_ISP_IQ_WDR_TYPE_t *data);
int MI_ISP_IQ_GetWDR(unsigned int DevId, unsigned int Channel, MI_ISP_IQ_WDR_TYPE_t *data);
int MI_ISP_AE_QueryExposureInfo(unsigned int DevId, unsigned int Channel, MI_ISP_AE_EXPO_INFO_TYPE_t *data);
int MI_ISP_AE_SetEVComp(unsigned int DevId, unsigned int Channel, MI_ISP_AE_EV_COMP_TYPE_t *data);
int MI_ISP_AE_SetExpoMode(unsigned int DevId, unsigned int Channel, MI_ISP_AE_MODE_TYPE_e *data);
int MI_ISP_AE_SetManualExpo(unsigned int DevId, unsigned int Channel, MI_ISP_AE_EXPO_VALUE_TYPE_t *data);
int MI_ISP_AE_GetManualExpo(unsigned int DevId, unsigned int Channel, MI_ISP_AE_EXPO_VALUE_TYPE_t *data);
int MI_ISP_AE_SetFlicker(unsigned int DevId, unsigned int Channel, MI_ISP_AE_FLICKER_TYPE_e *data);
int MI_ISP_AE_GetFlicker(unsigned int DevId, unsigned int Channel, MI_ISP_AE_FLICKER_TYPE_e *data);
int MI_ISP_AWB_QueryInfo(unsigned int DevId, unsigned int Channel, MI_ISP_AWB_QUERY_INFO_TYPE_t *data);
int MI_ISP_AWB_SetAttr(unsigned int DevId, unsigned int Channel, MI_ISP_AWB_ATTR_TYPE_t *data);
int MI_ISP_AWB_GetAttr(unsigned int DevId, unsigned int Channel, MI_ISP_AWB_ATTR_TYPE_t *data);
int MI_ISP_AWB_SetCTMwbAttr(unsigned int DevId, unsigned int Channel, MI_ISP_AWB_CTMWB_PARAM_t *data);

/* ---- ISP ---- */

typedef enum {
    E_MI_ISP_HDR_TYPE_OFF = 0,
} MI_ISP_HDRType_e;

typedef enum {
    E_MI_ISP_ROTATE_NONE = 0,
} MI_ISP_Rotate_e;

typedef struct {
    unsigned int u32DevStitchMask;
} MI_ISP_DevAttr_t;

typedef struct {
    unsigned int u32SensorBindId; /* bit n: sensor pad n */
    unsigned int _opaque_4[19];
} MI_ISP_ChannelAttr_t;

typedef struct {
    MI_ISP_HDRType_e eHDRType;
    unsigned int e3DNRLevel;
    unsigned char bMirror;
    unsigned char bFlip;
    MI_ISP_Rotate_e eRot;
} MI_ISP_ChnParam_t;

typedef struct {
    MI_SYS_WindowRect_t stCropRect;
    MI_SYS_PixelFormat_e ePixelFormat;
} MI_ISP_OutPortParam_t;

int MI_ISP_CreateDevice(unsigned int DevId, MI_ISP_DevAttr_t *pstDevAttr);
int MI_ISP_DestoryDevice(unsigned int DevId); /* the library's own spelling */
int MI_ISP_CreateChannel(unsigned int DevId, unsigned int ChnId, MI_ISP_ChannelAttr_t *pstChnAttr);
int MI_ISP_DestroyChannel(unsigned int DevId, unsigned int ChnId);
int MI_ISP_SetChnParam(unsigned int DevId, unsigned int ChnId, MI_ISP_ChnParam_t *pstChnParam);
int MI_ISP_StartChannel(unsigned int DevId, unsigned int ChnId);
int MI_ISP_StopChannel(unsigned int DevId, unsigned int ChnId);
int MI_ISP_GetInputPortCrop(unsigned int DevId, unsigned int ChnId, MI_SYS_WindowRect_t *pstCropInfo);
int MI_ISP_SetOutputPortParam(unsigned int DevId, unsigned int ChnId, unsigned int PortId,
                              MI_ISP_OutPortParam_t *pstOutPortParam);
int MI_ISP_EnableOutputPort(unsigned int DevId, unsigned int ChnId, unsigned int PortId);
int MI_ISP_DisableOutputPort(unsigned int DevId, unsigned int ChnId, unsigned int PortId);

/* ---- SCL ---- */

typedef enum {
    E_MI_SCL_FILTER_TYPE_AUTO = 0,
} MI_SCL_FilterType_e;

typedef struct {
    unsigned int u32NeedUseHWOutPortMask;
} MI_SCL_DevAttr_t;

typedef struct {
    unsigned int u32Reserved;
} MI_SCL_ChannelAttr_t;

typedef struct {
    MI_SYS_Rotate_e eRot;
} MI_SCL_ChnParam_t;

typedef struct {
    MI_SYS_WindowRect_t stSCLOutCropRect;
    MI_SYS_WindowSize_t stSCLOutputSize;
    unsigned char bMirror;
    unsigned char bFlip;
    MI_SYS_PixelFormat_e ePixelFormat;
    MI_SYS_CompressMode_e eCompressMode;
} MI_SCL_OutPortParam_t;

typedef struct {
    MI_SYS_PixelFormat_e ePixelFormat;
    unsigned int u32Width;
    unsigned int u32Height;
    MI_PHY phyAddr[2];
    unsigned int u32Stride[2];
    unsigned int u32BuffSize;
} MI_SCL_DirectBuf_t;

int MI_SCL_CreateDevice(unsigned int DevId, MI_SCL_DevAttr_t *pstSclDevAttr);
int MI_SCL_DestroyDevice(unsigned int DevId);
int MI_SCL_CreateChannel(unsigned int DevId, unsigned int ChnId, MI_SCL_ChannelAttr_t *pstChnAttr);
int MI_SCL_DestroyChannel(unsigned int DevId, unsigned int ChnId);
int MI_SCL_SetChnParam(unsigned int DevId, unsigned int ChnId, MI_SCL_ChnParam_t *pstChnParam);
int MI_SCL_StartChannel(unsigned int DevId, unsigned int ChnId);
int MI_SCL_StopChannel(unsigned int DevId, unsigned int ChnId);
int MI_SCL_SetOutputPortParam(unsigned int DevId, unsigned int ChnId, unsigned int PortId,
                              MI_SCL_OutPortParam_t *pstOutPortParam);
int MI_SCL_EnableOutputPort(unsigned int DevId, unsigned int ChnId, unsigned int PortId);
int MI_SCL_DisableOutputPort(unsigned int DevId, unsigned int ChnId, unsigned int PortId);
int MI_SCL_StretchBuf(MI_SCL_DirectBuf_t *pstSrcBuf, MI_SYS_WindowRect_t *pstSrcCrop, MI_SCL_DirectBuf_t *pstDstBuf,
                      MI_SCL_FilterType_e eFilterType);

/* ---- VENC ---- */

typedef enum {
    E_MI_VENC_MODTYPE_H264E = 2,
    E_MI_VENC_MODTYPE_H265E = 3,
    E_MI_VENC_MODTYPE_JPEGE = 4,
} MI_VENC_ModType_e;

typedef enum {
    E_MI_VENC_RC_MODE_H264CBR = 1,
    E_MI_VENC_RC_MODE_MJPEGFIXQP = 7,
    E_MI_VENC_RC_MODE_H265CBR = 8,
} MI_VENC_RcMode_e;

typedef enum {
    E_MI_VENC_INPUT_MODE_NORMAL_FRMBASE = 0,
    E_MI_VENC_INPUT_MODE_RING_ONE_FRM = 1,
} MI_VENC_InputSrcBufferMode_e;

typedef struct {
    unsigned int _opaque_0[2];
    unsigned char *pu8Addr;
    unsigned int u32Len;
    unsigned long long u64PTS;
    unsigned int _opaque_18[40];
} MI_VENC_Pack_t;

typedef struct {
    unsigned int _opaque_0[11];
    unsigned int eRefType;
    unsigned int _opaque_30[2];
} MI_VENC_StreamInfoH264_t;

typedef struct {
    unsigned int _opaque_0[9];
    unsigned int eRefType;
    unsigned int _opaque_28[2];
} MI_VENC_StreamInfoH265_t;

typedef struct {
    MI_VENC_Pack_t *pstPack;
    unsigned int u32PackCount;
    unsigned int _opaque_8[2];
    union {
        MI_VENC_StreamInfoH264_t stH264Info;
        MI_VENC_StreamInfoH265_t stH265Info;
    };
} MI_VENC_Stream_t;

typedef struct {
    unsigned int u32MaxPicWidth;
    unsigned int u32MaxPicHeight;
    unsigned int _opaque_8;
    unsigned int u32Profile;
    unsigned char bByFrame;
    unsigned int u32PicWidth;
    unsigned int u32PicHeight;
    unsigned int u32BFrameNum;
    unsigned int _opaque_20;
} MI_VENC_AttrH264_t;

typedef struct {
    unsigned int u32MaxPicWidth;
    unsigned int u32MaxPicHeight;
    unsigned int u32BufSize;
    unsigned char bByFrame;
    unsigned int u32PicWidth;
    unsigned int u32PicHeight;
    unsigned int _opaque_18[2];
} MI_VENC_AttrJpeg_t;

typedef struct {
    unsigned int u32MaxPicWidth;
    unsigned int u32MaxPicHeight;
    unsigned int _opaque_8;
    unsigned int u32Profile;
    unsigned char bByFrame;
    unsigned int u32PicWidth;
    unsigned int u32PicHeight;
    unsigned int u32BFrameNum;
    unsigned int _opaque_20;
} MI_VENC_AttrH265_t;

typedef struct {
    MI_VENC_ModType_e eType;
    union {
        MI_VENC_AttrH264_t stAttrH264e;
        MI_VENC_AttrJpeg_t stAttrJpeg;
        MI_VENC_AttrH265_t stAttrH265e;
    };
} MI_VENC_Attr_t;

typedef struct {
    unsigned int _opaque_0[2];
    unsigned int u32LeftStreamFrames;
    unsigned int _opaque_c;
    unsigned int u32CurPacks;
    unsigned int _opaque_14[5];
} MI_VENC_ChnStat_t;

typedef struct {
    unsigned int u32Qfactor;
    unsigned char au8YQt[64];
    unsigned char au8CbCrQt[64];
    unsigned int _opaque_84;
} MI_VENC_ParamJpeg_t;

typedef struct {
    unsigned int u32Gop;
    unsigned int u32StatTime;
    unsigned int u32SrcFrmRateNum;
    unsigned int u32SrcFrmRateDen;
    unsigned int u32BitRate;
    unsigned int u32FluctuateLevel;
} MI_VENC_AttrH264Cbr_t;

typedef struct {
    unsigned int u32SrcFrmRateNum;
    unsigned int u32SrcFrmRateDen;
    unsigned int u32Qfactor;
} MI_VENC_AttrMjpegFixQp_t;

typedef struct {
    unsigned int u32Gop;
    unsigned int u32StatTime;
    unsigned int u32SrcFrmRateNum;
    unsigned int u32SrcFrmRateDen;
    unsigned int u32BitRate;
    unsigned int u32FluctuateLevel;
} MI_VENC_AttrH265Cbr_t;

typedef struct {
    MI_VENC_RcMode_e eRcMode;
    union {
        MI_VENC_AttrH264Cbr_t stAttrH264Cbr;
        MI_VENC_AttrMjpegFixQp_t stAttrMjpegFixQp;
        MI_VENC_AttrH265Cbr_t stAttrH265Cbr;
        unsigned char _opaque[28];
    };
    unsigned int _opaque_20;
} MI_VENC_RcAttr_t;

typedef struct {
    MI_VENC_Attr_t stVeAttr;
    MI_VENC_RcAttr_t stRcAttr;
} MI_VENC_ChnAttr_t;

typedef struct {
    MI_VENC_InputSrcBufferMode_e eInputSrcBufferMode;
} MI_VENC_InputSourceConfig_t;

/* MI_VENC_RcParam_t with its union's CBR member, H.264 and H.265 alike (the header's RC_TEXTURE_THR_SIZE is 1) */
typedef struct {
    unsigned int u32MaxQp;
    unsigned int u32MinQp;
    int s32IPQPDelta;
    unsigned int u32MaxIQp;
    unsigned int u32MinIQp;
    unsigned int u32MaxIPProp;
    unsigned int u32MaxISize;
    unsigned int u32MaxPSize;
} MI_VENC_ParamH26xCbr_t;

typedef struct {
    unsigned int au32ThrdI[1];
    unsigned int au32ThrdP[1];
    unsigned int u32RowQpDelta;
    union {
        MI_VENC_ParamH26xCbr_t stParamCbr;
        unsigned char _opaque[40];
    };
    void *pRcParam;
} MI_VENC_RcParam_t;

int MI_VENC_CreateChn(int VeDev, int VeChn, MI_VENC_ChnAttr_t *pstAttr);
int MI_VENC_DestroyChn(int VeDev, int VeChn);
int MI_VENC_StartRecvPic(int VeDev, int VeChn);
int MI_VENC_StopRecvPic(int VeDev, int VeChn);
int MI_VENC_Query(int VeDev, int VeChn, MI_VENC_ChnStat_t *pstStat);
int MI_VENC_GetStream(int VeDev, int VeChn, MI_VENC_Stream_t *pstStream, int s32MilliSec);
int MI_VENC_ReleaseStream(int VeDev, int VeChn, MI_VENC_Stream_t *pstStream);
int MI_VENC_RequestIdr(int VeDev, int VeChn, unsigned char bInstant);
int MI_VENC_SetJpegParam(int VeDev, int VeChn, MI_VENC_ParamJpeg_t *pstJpegParam);
int MI_VENC_SetInputSourceConfig(int VeDev, int VeChn, MI_VENC_InputSourceConfig_t *pstInputSourceConfig);
int MI_VENC_SetRcParam(int VeDev, int VeChn, MI_VENC_RcParam_t *pstRcParam);
int MI_VENC_GetRcParam(int VeDev, int VeChn, MI_VENC_RcParam_t *pstRcParam);

/* ---- DISP ---- */

typedef enum {
    E_MI_DISP_INTF_HDMI = 5,
} MI_DISP_Interface_e;

typedef enum {
    E_MI_DISP_OUTPUT_480P60 = 6,
    E_MI_DISP_OUTPUT_576P50 = 7,
    E_MI_DISP_OUTPUT_720P50 = 8,
    E_MI_DISP_OUTPUT_720P60 = 9,
    E_MI_DISP_OUTPUT_1080P24 = 10,
    E_MI_DISP_OUTPUT_1080P25 = 11,
    E_MI_DISP_OUTPUT_1080P30 = 12,
    E_MI_DISP_OUTPUT_1080P50 = 15,
    E_MI_DISP_OUTPUT_1080P60 = 16,
    E_MI_DISP_OUTPUT_2560x1440_30 = 28,
    E_MI_DISP_OUTPUT_2560x1440_50 = 29,
    E_MI_DISP_OUTPUT_2560x1440_60 = 30,
    E_MI_DISP_OUTPUT_3840x2160_25 = 32,
    E_MI_DISP_OUTPUT_3840x2160_30 = 33,
} MI_DISP_OutputTiming_e;

typedef enum {
    E_MI_DISP_SYNC_MODE_FREE_RUN = 2,
} MI_DISP_SyncMode_e;

typedef struct {
    unsigned int u32BgColor;
    MI_DISP_Interface_e eIntfType;
    MI_DISP_OutputTiming_e eIntfSync;
    unsigned int _opaque_c[10];
} MI_DISP_PubAttr_t;

typedef struct {
    unsigned short u16X;
    unsigned short u16Y;
    unsigned short u16Width;
    unsigned short u16Height;
} MI_DISP_VidWinRect_t;

typedef struct {
    unsigned int u16Width;
    unsigned int u16Height;
} MI_DISP_VideoLayerSize_t;

typedef struct {
    MI_DISP_VidWinRect_t stVidLayerDispWin;
    MI_DISP_VideoLayerSize_t stVidLayerSize;
    MI_SYS_PixelFormat_e ePixFormat;
} MI_DISP_VideoLayerAttr_t;

typedef struct {
    MI_DISP_VidWinRect_t stDispWin;
    unsigned short u16SrcWidth;
    unsigned short u16SrcHeight;
} MI_DISP_InputPortAttr_t;

int MI_DISP_Enable(int DispDev);
int MI_DISP_Disable(int DispDev);
int MI_DISP_SetPubAttr(int DispDev, const MI_DISP_PubAttr_t *pstPubAttr);
int MI_DISP_EnableVideoLayer(int DispLayer);
int MI_DISP_DisableVideoLayer(int DispLayer);
int MI_DISP_SetVideoLayerAttr(int DispLayer, const MI_DISP_VideoLayerAttr_t *pstLayerAttr);
int MI_DISP_BindVideoLayer(int DispLayer, int DispDev);
int MI_DISP_UnBindVideoLayer(int DispLayer, int DispDev);
int MI_DISP_SetInputPortAttr(int DispLayer, unsigned int LayerInputPort,
                             const MI_DISP_InputPortAttr_t *pstInputPortAttr);
int MI_DISP_EnableInputPort(int DispLayer, unsigned int LayerInputPort);
int MI_DISP_DisableInputPort(int DispLayer, unsigned int LayerInputPort);
int MI_DISP_SetInputPortSyncMode(int DispLayer, unsigned int LayerInputPort, MI_DISP_SyncMode_e eMode);

/* ---- HDMI ---- */

typedef enum {
    E_MI_HDMI_ID_0 = 0,
} MI_HDMI_DeviceId_e;

typedef enum {
    E_MI_HDMI_EVENT_HOTPLUG = 0,
    E_MI_HDMI_EVENT_NO_PLUG = 1,
} MI_HDMI_EventType_e;

typedef enum {
    E_MI_HDMI_TIMING_480_60P = 1,
    E_MI_HDMI_TIMING_576_50P = 3,
    E_MI_HDMI_TIMING_720_50P = 4,
    E_MI_HDMI_TIMING_720_60P = 5,
    E_MI_HDMI_TIMING_1080_50P = 7,
    E_MI_HDMI_TIMING_1080_60P = 9,
    E_MI_HDMI_TIMING_1080_30P = 10,
    E_MI_HDMI_TIMING_1080_25P = 11,
    E_MI_HDMI_TIMING_1080_24P = 12,
    E_MI_HDMI_TIMING_4K2K_30P = 13,
    E_MI_HDMI_TIMING_1440_50P = 14,
    E_MI_HDMI_TIMING_1440_60P = 15,
    E_MI_HDMI_TIMING_1440_30P = 17,
    E_MI_HDMI_TIMING_4K2K_25P = 24,
} MI_HDMI_TimingType_e;

typedef enum {
    E_MI_HDMI_COLOR_TYPE_YCBCR444 = 2,
} MI_HDMI_ColorType_e;

typedef enum {
    E_MI_HDMI_OUTPUT_MODE_HDMI = 0,
} MI_HDMI_OutputMode_e;

typedef enum {
    E_MI_HDMI_DEEP_COLOR_24BIT = 0,
} MI_HDMI_DeepColor_e;

typedef int (*MI_HDMI_EventCallBack)(MI_HDMI_DeviceId_e eHdmi, MI_HDMI_EventType_e event, void *pEventParam,
                                     void *pUsrParam);

typedef struct {
    MI_HDMI_EventCallBack pfnHdmiEventCallback;
    void *pCallBackArgs;
} MI_HDMI_InitParam_t;

typedef struct {
    unsigned char bEnableVideo;
    MI_HDMI_TimingType_e eTimingType;
    MI_HDMI_OutputMode_e eOutputMode;
    MI_HDMI_ColorType_e eColorType;
    unsigned int _opaque_10;
    MI_HDMI_DeepColor_e eDeepColorMode;
} MI_HDMI_VideoAttr_t;

typedef struct {
    unsigned char bEnableAudio;
    unsigned char _opaque_1[3];
    unsigned int _opaque_4[3];
} MI_HDMI_AudioAttr_t;

typedef struct {
    unsigned char bConnect;
    MI_HDMI_VideoAttr_t stVideoAttr;
    MI_HDMI_AudioAttr_t stAudioAttr;
    unsigned int _opaque_2c;
} MI_HDMI_Attr_t;

typedef struct {
    unsigned char bEdidValid;
    unsigned int u32Edidlength;
    unsigned char au8Edid[512];
} MI_HDMI_Edid_t;

int MI_HDMI_Init(MI_HDMI_InitParam_t *pstInitParam);
int MI_HDMI_Open(MI_HDMI_DeviceId_e eHdmi);
int MI_HDMI_Close(MI_HDMI_DeviceId_e eHdmi);
int MI_HDMI_SetAttr(MI_HDMI_DeviceId_e eHdmi, MI_HDMI_Attr_t *pstAttr);
int MI_HDMI_GetAttr(MI_HDMI_DeviceId_e eHdmi, MI_HDMI_Attr_t *pstAttr);
int MI_HDMI_Start(MI_HDMI_DeviceId_e eHdmi);
int MI_HDMI_Stop(MI_HDMI_DeviceId_e eHdmi);
int MI_HDMI_ForceGetEdid(MI_HDMI_DeviceId_e eHdmi, MI_HDMI_Edid_t *pstEdidData);

/* ---- RGN ---- */

typedef enum {
    E_MI_RGN_TYPE_OSD = 0,
    E_MI_RGN_TYPE_COVER = 1,
} MI_RGN_Type_e;

typedef enum {
    E_MI_RGN_PIXEL_FORMAT_ARGB4444 = 1,
} MI_RGN_PixelFormat_e;

typedef enum {
    E_MI_RGN_PIXEL_ALPHA = 0,
    E_MI_RGN_CONSTANT_ALPHA = 1,
} MI_RGN_AlphaMode_e;

typedef struct {
    MI_RGN_Type_e eType;
    MI_RGN_PixelFormat_e ePixelFmt;
    unsigned int u32Width;
    unsigned int u32Height;
} MI_RGN_Attr_t;

typedef struct {
    MI_ModuleId_e eModId;
    int s32DevId;
    int s32ChnId;
    int s32PortId;
} MI_RGN_ChnPort_t;

typedef struct {
    unsigned int u32X;
    unsigned int u32Y;
} MI_RGN_Point_t;

typedef struct {
    unsigned int u32Width;
    unsigned int u32Height;
} MI_RGN_Size_t;

typedef struct {
    unsigned int u32Layer;
    MI_RGN_AlphaMode_e eAlphaMode;
    unsigned char u8BgAlpha;
    unsigned char u8FgAlpha;
    unsigned char _opaque_a[2];
    unsigned int _opaque_c[4];
} MI_RGN_OsdChnPortParam_t;

typedef struct {
    unsigned int u32Layer;
    MI_RGN_Size_t stSize;
    unsigned int u32Color;
} MI_RGN_CoverChnPortParam_t;

typedef struct {
    unsigned char bShow;
    MI_RGN_Point_t stPoint;
    union {
        MI_RGN_OsdChnPortParam_t stOsdChnPort;
        MI_RGN_CoverChnPortParam_t stCoverChnPort;
    } unPara;
} MI_RGN_ChnPortParam_t;

typedef struct {
    MI_PHY phyAddr;
    unsigned int _opaque_8[3];
    unsigned int u32Stride;
    unsigned int _opaque_18[2];
} MI_RGN_CanvasInfo_t;

typedef struct {
    unsigned char _opaque_0[1024];
} MI_RGN_PaletteTable_t;

int MI_RGN_Init(unsigned short u16SocId, MI_RGN_PaletteTable_t *pstPaletteTable);
int MI_RGN_DeInit(unsigned short u16SocId);
int MI_RGN_Create(unsigned short u16SocId, unsigned int hHandle, MI_RGN_Attr_t *pstRegion);
int MI_RGN_Destroy(unsigned short u16SocId, unsigned int hHandle);
int MI_RGN_AttachToChn(unsigned short u16SocId, unsigned int hHandle, MI_RGN_ChnPort_t *pstChnPort,
                       MI_RGN_ChnPortParam_t *pstChnAttr);
int MI_RGN_SetDisplayAttr(unsigned short u16SocId, unsigned int hHandle, MI_RGN_ChnPort_t *pstChnPort,
                          MI_RGN_ChnPortParam_t *pstChnAttr);
int MI_RGN_GetCanvasInfo(unsigned short u16SocId, unsigned int hHandle, MI_RGN_CanvasInfo_t *pstCanvasInfo);
int MI_RGN_UpdateCanvas(unsigned short u16SocId, unsigned int hHandle);

/* ---- VDEC ---- */

typedef enum {
    E_MI_VDEC_CODEC_TYPE_H264 = 0,
    E_MI_VDEC_CODEC_TYPE_H265 = 1,
} MI_VDEC_CodecType_e;

typedef enum {
    E_MI_VDEC_VIDEO_MODE_FRAME = 1,
} MI_VDEC_VideoMode_e;

typedef enum {
    E_MI_VDEC_DPB_MODE_NORMAL = 0,
} MI_VDEC_DpbBufMode_e;

typedef struct {
    unsigned int u32Reserved;
} MI_VDEC_InitParam_t;

typedef struct {
    unsigned int u32RefFrameNum;
    unsigned int _opaque_4;
} MI_VDEC_VideoAttr_t;

typedef struct {
    MI_VDEC_CodecType_e eCodecType;
    unsigned int u32BufSize;
    unsigned int _opaque_8;
    unsigned int u32PicWidth;
    unsigned int u32PicHeight;
    MI_VDEC_VideoMode_e eVideoMode;
    MI_VDEC_DpbBufMode_e eDpbBufMode;
    MI_VDEC_VideoAttr_t stVdecVideoAttr;
} MI_VDEC_ChnAttr_t;

typedef struct {
    unsigned char *pu8Addr;
    unsigned int u32Len;
    unsigned long long u64PTS;
    unsigned char bEndOfFrame;
    unsigned char bEndOfStream;
} MI_VDEC_VideoStream_t;

typedef struct {
    unsigned int _opaque_0[2];
    unsigned int u32LeftStreamFrames;
    unsigned int _opaque_c;
    unsigned int u32RecvStreamFrames;
    unsigned int u32DecodeStreamFrames;
    unsigned int _opaque_18[2];
} MI_VDEC_ChnStat_t;

typedef struct {
    unsigned short u16Width;
    unsigned short u16Height;
} MI_VDEC_OutputPortAttr_t;

int MI_VDEC_CreateDev(int VdecDev, MI_VDEC_InitParam_t *pstInitParam);
int MI_VDEC_CreateChn(int VdecDev, int VdecChn, MI_VDEC_ChnAttr_t *pstAttr);
int MI_VDEC_DestroyChn(int VdecDev, int VdecChn);
int MI_VDEC_StartChn(int VdecDev, int VdecChn);
int MI_VDEC_StopChn(int VdecDev, int VdecChn);
int MI_VDEC_GetChnStat(int VdecDev, int VdecChn, MI_VDEC_ChnStat_t *pstStat);
int MI_VDEC_SendStream(int VdecDev, int VdecChn, MI_VDEC_VideoStream_t *pstStream, int s32MilliSec);
int MI_VDEC_PauseChn(int VdecDev, int VdecChn);
int MI_VDEC_RefreshChn(int VdecDev, int VdecChn);
int MI_VDEC_ResumeChn(int VdecDev, int VdecChn);
int MI_VDEC_SetOutputPortAttr(int VdecDev, int VdecChn, MI_VDEC_OutputPortAttr_t *pstAttr);

/* ---- CUS3A ---- */

/* status[0], [1], [2]: AE, AWB, AF ready (1) */
void CUS3A_GetAlgoStatus(unsigned int DevId, unsigned int ChnId, unsigned int *pu32Status);

#endif
