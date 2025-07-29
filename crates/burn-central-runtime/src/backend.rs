#![allow(unused)]

use burn::backend::Autodiff;
use burn::prelude::{Backend, Device, Shape, TensorData};
use burn::tensor::backend::{DeviceId, DeviceOps};
use burn::tensor::ops::{
    ActivationOps, BoolTensor, BoolTensorOps, ConvOptions, ConvTransposeOptions,
    DeformConv2dBackward, DeformConvOptions, FloatElem, FloatTensor, FloatTensorOps, IntElem,
    IntTensor, IntTensorOps, InterpolateOptions, MaxPool2dBackward, MaxPool2dWithIndices,
    ModuleOps, QTensorOps, QuantizedTensor, TransactionOps,
};
use burn::tensor::quantization::{
    QTensorPrimitive, QuantizationMode, QuantizationParametersPrimitive, QuantizationScheme,
    QuantizationType,
};
use burn::tensor::{DType, Distribution, FloatDType, TensorMetadata};
use std::ops::Range;

#[cfg(not(feature = "ndarray-stub"))]
pub type AutodiffBackendStub = Autodiff<BackendStub>;
#[cfg(feature = "ndarray-stub")]
pub type AutodiffBackendStub = Autodiff<burn::backend::NdArray>;

#[derive(Clone, Debug, Default)]
pub struct BackendStub;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeviceStub;

#[derive(Clone, Debug)]
pub struct TensorStub;

impl TensorMetadata for TensorStub {
    fn dtype(&self) -> DType {
        DType::F32
    }

    fn shape(&self) -> Shape {
        Shape::new([1, 1, 1, 1])
    }
}

impl QTensorPrimitive for TensorStub {
    fn scheme(&self) -> &QuantizationScheme {
        &QuantizationScheme::PerTensor(QuantizationMode::Symmetric, QuantizationType::QInt8)
    }
}

impl DeviceOps for DeviceStub {
    fn id(&self) -> DeviceId {
        DeviceId {
            type_id: 0,
            index_id: 0,
        }
    }
}

impl FloatTensorOps<Self> for BackendStub {
    fn float_from_data(data: TensorData, device: &Device<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_random(
        shape: Shape,
        distribution: Distribution,
        device: &Device<Self>,
    ) -> FloatTensor<Self> {
        TensorStub
    }

    async fn float_into_data(tensor: FloatTensor<Self>) -> TensorData {
        TensorData::new::<<Self as Backend>::FloatElem, _>(Default::default(), [])
    }

    fn float_device(tensor: &FloatTensor<Self>) -> Device<Self> {
        DeviceStub
    }

    fn float_to_device(tensor: FloatTensor<Self>, device: &Device<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_into_int(tensor: FloatTensor<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn float_empty(shape: Shape, device: &Device<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_add(lhs: FloatTensor<Self>, rhs: FloatTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_add_scalar(lhs: FloatTensor<Self>, rhs: FloatElem<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_sub(lhs: FloatTensor<Self>, rhs: FloatTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_sub_scalar(lhs: FloatTensor<Self>, rhs: FloatElem<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_mul(lhs: FloatTensor<Self>, rhs: FloatTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_mul_scalar(lhs: FloatTensor<Self>, rhs: FloatElem<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_div(lhs: FloatTensor<Self>, rhs: FloatTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_div_scalar(lhs: FloatTensor<Self>, rhs: FloatElem<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_remainder(lhs: FloatTensor<Self>, rhs: FloatTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_remainder_scalar(lhs: FloatTensor<Self>, rhs: FloatElem<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_matmul(lhs: FloatTensor<Self>, rhs: FloatTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_recip(tensor: FloatTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_swap_dims(tensor: FloatTensor<Self>, dim1: usize, dim2: usize) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_permute(tensor: FloatTensor<Self>, axes: &[usize]) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_flip(tensor: FloatTensor<Self>, axes: &[usize]) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_reshape(tensor: FloatTensor<Self>, shape: Shape) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_gather(
        dim: usize,
        tensor: FloatTensor<Self>,
        indices: IntTensor<Self>,
    ) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_scatter(
        dim: usize,
        tensor: FloatTensor<Self>,
        indices: IntTensor<Self>,
        value: FloatTensor<Self>,
    ) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_select(
        tensor: FloatTensor<Self>,
        dim: usize,
        indices: IntTensor<Self>,
    ) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_select_assign(
        tensor: FloatTensor<Self>,
        dim: usize,
        indices: IntTensor<Self>,
        value: FloatTensor<Self>,
    ) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_slice(tensor: FloatTensor<Self>, ranges: &[Range<usize>]) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_slice_assign(
        tensor: FloatTensor<Self>,
        ranges: &[Range<usize>],
        value: FloatTensor<Self>,
    ) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_mask_where(
        tensor: FloatTensor<Self>,
        mask: BoolTensor<Self>,
        value: FloatTensor<Self>,
    ) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_mask_fill(
        tensor: FloatTensor<Self>,
        mask: BoolTensor<Self>,
        value: FloatElem<Self>,
    ) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_equal(lhs: FloatTensor<Self>, rhs: FloatTensor<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn float_equal_elem(lhs: FloatTensor<Self>, rhs: FloatElem<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn float_greater(lhs: FloatTensor<Self>, rhs: FloatTensor<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn float_greater_elem(lhs: FloatTensor<Self>, rhs: FloatElem<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn float_greater_equal(lhs: FloatTensor<Self>, rhs: FloatTensor<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn float_greater_equal_elem(lhs: FloatTensor<Self>, rhs: FloatElem<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn float_lower(lhs: FloatTensor<Self>, rhs: FloatTensor<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn float_lower_elem(lhs: FloatTensor<Self>, rhs: FloatElem<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn float_lower_equal(lhs: FloatTensor<Self>, rhs: FloatTensor<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn float_lower_equal_elem(lhs: FloatTensor<Self>, rhs: FloatElem<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn float_sum(tensor: FloatTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_sum_dim(tensor: FloatTensor<Self>, dim: usize) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_mean_dim(tensor: FloatTensor<Self>, dim: usize) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_cast(tensor: FloatTensor<Self>, dtype: FloatDType) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_exp(tensor: FloatTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_log(tensor: FloatTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_log1p(tensor: FloatTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_powf(lhs: FloatTensor<Self>, rhs: FloatTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_powf_scalar(tensor: FloatTensor<Self>, value: f32) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_sqrt(tensor: FloatTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_abs(tensor: FloatTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_cos(tensor: FloatTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_sin(tensor: FloatTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_round(tensor: FloatTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_floor(tensor: FloatTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_ceil(tensor: FloatTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_erf(tensor: FloatTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn float_argmax(tensor: FloatTensor<Self>, dim: usize) -> IntTensor<Self> {
        TensorStub
    }

    fn float_argmin(tensor: FloatTensor<Self>, dim: usize) -> IntTensor<Self> {
        TensorStub
    }

    fn float_expand(tensor: FloatTensor<Self>, shape: Shape) -> FloatTensor<Self> {
        TensorStub
    }
}

impl BoolTensorOps<Self> for BackendStub {
    fn bool_empty(shape: Shape, device: &Device<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    async fn bool_into_data(tensor: BoolTensor<Self>) -> TensorData {
        TensorData::new::<<Self as Backend>::BoolElem, _>(Default::default(), [])
    }

    fn bool_from_data(data: TensorData, device: &Device<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn bool_into_int(tensor: BoolTensor<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn bool_into_float(tensor: BoolTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn bool_device(tensor: &BoolTensor<Self>) -> Device<Self> {
        DeviceStub
    }

    fn bool_to_device(tensor: BoolTensor<Self>, device: &Device<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn bool_reshape(tensor: BoolTensor<Self>, shape: Shape) -> BoolTensor<Self> {
        TensorStub
    }

    fn bool_slice(tensor: BoolTensor<Self>, ranges: &[Range<usize>]) -> BoolTensor<Self> {
        TensorStub
    }

    fn bool_slice_assign(
        tensor: BoolTensor<Self>,
        ranges: &[Range<usize>],
        value: BoolTensor<Self>,
    ) -> BoolTensor<Self> {
        TensorStub
    }

    fn bool_equal(lhs: BoolTensor<Self>, rhs: BoolTensor<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn bool_not(tensor: BoolTensor<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn bool_and(tensor: BoolTensor<Self>, rhs: BoolTensor<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn bool_or(tensor: BoolTensor<Self>, rhs: BoolTensor<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn bool_swap_dims(tensor: BoolTensor<Self>, dim1: usize, dim2: usize) -> BoolTensor<Self> {
        TensorStub
    }

    fn bool_permute(tensor: BoolTensor<Self>, axes: &[usize]) -> BoolTensor<Self> {
        TensorStub
    }

    fn bool_flip(tensor: BoolTensor<Self>, axes: &[usize]) -> BoolTensor<Self> {
        TensorStub
    }

    fn bool_expand(tensor: BoolTensor<Self>, shape: Shape) -> BoolTensor<Self> {
        TensorStub
    }
}

impl IntTensorOps<Self> for BackendStub {
    fn int_empty(shape: Shape, device: &Device<Self>) -> IntTensor<Self> {
        TensorStub
    }

    async fn int_into_data(tensor: IntTensor<Self>) -> TensorData {
        TensorData::new::<<Self as Backend>::IntElem, _>(Default::default(), [])
    }

    fn int_from_data(data: TensorData, device: &Device<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn int_device(tensor: &IntTensor<Self>) -> Device<Self> {
        DeviceStub
    }

    fn int_to_device(tensor: IntTensor<Self>, device: &Device<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn int_reshape(tensor: IntTensor<Self>, shape: Shape) -> IntTensor<Self> {
        TensorStub
    }

    fn int_slice(tensor: IntTensor<Self>, indices: &[Range<usize>]) -> IntTensor<Self> {
        TensorStub
    }

    fn int_slice_assign(
        tensor: IntTensor<Self>,
        indices: &[Range<usize>],
        value: IntTensor<Self>,
    ) -> IntTensor<Self> {
        TensorStub
    }

    fn int_into_float(tensor: IntTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn int_mask_where(
        tensor: IntTensor<Self>,
        mask: BoolTensor<Self>,
        source: IntTensor<Self>,
    ) -> IntTensor<Self> {
        TensorStub
    }

    fn int_mask_fill(
        tensor: IntTensor<Self>,
        mask: BoolTensor<Self>,
        value: IntElem<Self>,
    ) -> IntTensor<Self> {
        TensorStub
    }

    fn int_gather(
        dim: usize,
        tensor: IntTensor<Self>,
        indices: IntTensor<Self>,
    ) -> IntTensor<Self> {
        TensorStub
    }

    fn int_scatter(
        dim: usize,
        tensor: IntTensor<Self>,
        indices: IntTensor<Self>,
        value: IntTensor<Self>,
    ) -> IntTensor<Self> {
        TensorStub
    }

    fn int_select(
        tensor: IntTensor<Self>,
        dim: usize,
        indices: IntTensor<Self>,
    ) -> IntTensor<Self> {
        TensorStub
    }

    fn int_select_assign(
        tensor: IntTensor<Self>,
        dim: usize,
        indices: IntTensor<Self>,
        value: IntTensor<Self>,
    ) -> IntTensor<Self> {
        TensorStub
    }

    fn int_equal(lhs: IntTensor<Self>, rhs: IntTensor<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn int_equal_elem(lhs: IntTensor<Self>, rhs: IntElem<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn int_greater(lhs: IntTensor<Self>, rhs: IntTensor<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn int_greater_elem(lhs: IntTensor<Self>, rhs: IntElem<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn int_greater_equal(lhs: IntTensor<Self>, rhs: IntTensor<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn int_greater_equal_elem(lhs: IntTensor<Self>, rhs: IntElem<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn int_lower(lhs: IntTensor<Self>, rhs: IntTensor<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn int_lower_elem(lhs: IntTensor<Self>, rhs: IntElem<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn int_lower_equal(lhs: IntTensor<Self>, rhs: IntTensor<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn int_lower_equal_elem(lhs: IntTensor<Self>, rhs: IntElem<Self>) -> BoolTensor<Self> {
        TensorStub
    }

    fn int_add(lhs: IntTensor<Self>, rhs: IntTensor<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn int_add_scalar(lhs: IntTensor<Self>, rhs: IntElem<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn int_sub(lhs: IntTensor<Self>, rhs: IntTensor<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn int_sub_scalar(lhs: IntTensor<Self>, rhs: IntElem<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn int_mul(lhs: IntTensor<Self>, rhs: IntTensor<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn int_mul_scalar(lhs: IntTensor<Self>, rhs: IntElem<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn int_div(lhs: IntTensor<Self>, rhs: IntTensor<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn int_div_scalar(lhs: IntTensor<Self>, rhs: IntElem<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn int_remainder(lhs: IntTensor<Self>, rhs: IntTensor<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn int_remainder_scalar(lhs: IntTensor<Self>, rhs: IntElem<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn int_zeros(shape: Shape, device: &Device<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn int_ones(shape: Shape, device: &Device<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn int_sum(tensor: IntTensor<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn int_sum_dim(tensor: IntTensor<Self>, dim: usize) -> IntTensor<Self> {
        TensorStub
    }

    fn int_prod(tensor: IntTensor<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn int_prod_dim(tensor: IntTensor<Self>, dim: usize) -> IntTensor<Self> {
        TensorStub
    }

    fn int_mean_dim(tensor: IntTensor<Self>, dim: usize) -> IntTensor<Self> {
        TensorStub
    }

    fn int_argmax(tensor: IntTensor<Self>, dim: usize) -> IntTensor<Self> {
        TensorStub
    }

    fn int_argmin(tensor: IntTensor<Self>, dim: usize) -> IntTensor<Self> {
        TensorStub
    }

    fn int_abs(tensor: IntTensor<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn int_swap_dims(tensor: IntTensor<Self>, dim1: usize, dim2: usize) -> IntTensor<Self> {
        TensorStub
    }

    fn int_permute(tensor: IntTensor<Self>, axes: &[usize]) -> IntTensor<Self> {
        TensorStub
    }

    fn int_flip(tensor: IntTensor<Self>, axes: &[usize]) -> IntTensor<Self> {
        TensorStub
    }

    fn int_random(
        shape: Shape,
        distribution: Distribution,
        device: &Device<Self>,
    ) -> IntTensor<Self> {
        TensorStub
    }

    fn int_expand(tensor: IntTensor<Self>, shape: Shape) -> IntTensor<Self> {
        TensorStub
    }

    fn bitwise_and(lhs: IntTensor<Self>, rhs: IntTensor<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn bitwise_and_scalar(lhs: IntTensor<Self>, rhs: IntElem<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn bitwise_or(lhs: IntTensor<Self>, rhs: IntTensor<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn bitwise_or_scalar(lhs: IntTensor<Self>, rhs: IntElem<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn bitwise_xor(lhs: IntTensor<Self>, rhs: IntTensor<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn bitwise_xor_scalar(lhs: IntTensor<Self>, rhs: IntElem<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn bitwise_not(tensor: IntTensor<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn bitwise_left_shift(lhs: IntTensor<Self>, rhs: IntTensor<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn bitwise_left_shift_scalar(lhs: IntTensor<Self>, rhs: IntElem<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn bitwise_right_shift(lhs: IntTensor<Self>, rhs: IntTensor<Self>) -> IntTensor<Self> {
        TensorStub
    }

    fn bitwise_right_shift_scalar(lhs: IntTensor<Self>, rhs: IntElem<Self>) -> IntTensor<Self> {
        TensorStub
    }
}

impl ModuleOps<Self> for BackendStub {
    fn conv2d(
        x: FloatTensor<Self>,
        weight: FloatTensor<Self>,
        bias: Option<FloatTensor<Self>>,
        options: ConvOptions<2>,
    ) -> FloatTensor<Self> {
        TensorStub
    }

    fn deform_conv2d(
        x: FloatTensor<Self>,
        offset: FloatTensor<Self>,
        weight: FloatTensor<Self>,
        mask: Option<FloatTensor<Self>>,
        bias: Option<FloatTensor<Self>>,
        options: DeformConvOptions<2>,
    ) -> FloatTensor<Self> {
        TensorStub
    }

    fn deform_conv2d_backward(
        x: FloatTensor<Self>,
        offset: FloatTensor<Self>,
        weight: FloatTensor<Self>,
        mask: Option<FloatTensor<Self>>,
        bias: Option<FloatTensor<Self>>,
        output_grad: FloatTensor<Self>,
        options: DeformConvOptions<2>,
    ) -> DeformConv2dBackward<Self> {
        DeformConv2dBackward::new(
            TensorStub,
            TensorStub,
            TensorStub,
            Some(TensorStub),
            Some(TensorStub),
        )
    }

    fn conv3d(
        x: FloatTensor<Self>,
        weight: FloatTensor<Self>,
        bias: Option<FloatTensor<Self>>,
        options: ConvOptions<3>,
    ) -> FloatTensor<Self> {
        TensorStub
    }

    fn conv_transpose2d(
        x: FloatTensor<Self>,
        weight: FloatTensor<Self>,
        bias: Option<FloatTensor<Self>>,
        options: ConvTransposeOptions<2>,
    ) -> FloatTensor<Self> {
        TensorStub
    }

    fn conv_transpose3d(
        x: FloatTensor<Self>,
        weight: FloatTensor<Self>,
        bias: Option<FloatTensor<Self>>,
        options: ConvTransposeOptions<3>,
    ) -> FloatTensor<Self> {
        TensorStub
    }

    fn avg_pool2d(
        x: FloatTensor<Self>,
        kernel_size: [usize; 2],
        stride: [usize; 2],
        padding: [usize; 2],
        count_include_pad: bool,
    ) -> FloatTensor<Self> {
        TensorStub
    }

    fn avg_pool2d_backward(
        x: FloatTensor<Self>,
        grad: FloatTensor<Self>,
        kernel_size: [usize; 2],
        stride: [usize; 2],
        padding: [usize; 2],
        count_include_pad: bool,
    ) -> FloatTensor<Self> {
        TensorStub
    }

    fn adaptive_avg_pool2d(x: FloatTensor<Self>, output_size: [usize; 2]) -> FloatTensor<Self> {
        TensorStub
    }

    fn adaptive_avg_pool2d_backward(
        x: FloatTensor<Self>,
        grad: FloatTensor<Self>,
    ) -> FloatTensor<Self> {
        TensorStub
    }

    fn max_pool2d(
        x: FloatTensor<Self>,
        kernel_size: [usize; 2],
        stride: [usize; 2],
        padding: [usize; 2],
        dilation: [usize; 2],
    ) -> FloatTensor<Self> {
        TensorStub
    }

    fn max_pool2d_with_indices(
        x: FloatTensor<Self>,
        kernel_size: [usize; 2],
        stride: [usize; 2],
        padding: [usize; 2],
        dilation: [usize; 2],
    ) -> MaxPool2dWithIndices<Self> {
        MaxPool2dWithIndices::new(TensorStub, TensorStub)
    }

    fn max_pool2d_with_indices_backward(
        x: FloatTensor<Self>,
        kernel_size: [usize; 2],
        stride: [usize; 2],
        padding: [usize; 2],
        dilation: [usize; 2],
        output_grad: FloatTensor<Self>,
        indices: IntTensor<Self>,
    ) -> MaxPool2dBackward<Self> {
        MaxPool2dBackward::new(TensorStub)
    }

    fn interpolate(
        x: FloatTensor<Self>,
        output_size: [usize; 2],
        options: InterpolateOptions,
    ) -> FloatTensor<Self> {
        TensorStub
    }

    fn interpolate_backward(
        x: FloatTensor<Self>,
        grad: FloatTensor<Self>,
        output_size: [usize; 2],
        options: InterpolateOptions,
    ) -> FloatTensor<Self> {
        TensorStub
    }
}

impl ActivationOps<Self> for BackendStub {}

impl QTensorOps<Self> for BackendStub {
    fn q_from_data(data: TensorData, device: &Device<Self>) -> QuantizedTensor<Self> {
        TensorStub
    }

    fn quantize(
        tensor: FloatTensor<Self>,
        scheme: &QuantizationScheme,
        qparams: QuantizationParametersPrimitive<Self>,
    ) -> QuantizedTensor<Self> {
        TensorStub
    }

    fn dequantize(tensor: QuantizedTensor<Self>) -> FloatTensor<Self> {
        TensorStub
    }

    fn q_device(tensor: &QuantizedTensor<Self>) -> Device<Self> {
        DeviceStub
    }

    fn q_to_device(tensor: QuantizedTensor<Self>, device: &Device<Self>) -> QuantizedTensor<Self> {
        TensorStub
    }

    fn q_reshape(tensor: QuantizedTensor<Self>, shape: Shape) -> QuantizedTensor<Self> {
        TensorStub
    }

    async fn q_into_data(tensor: QuantizedTensor<Self>) -> TensorData {
        TensorData::new::<<Self as Backend>::QuantizedEncoding, _>(Default::default(), [])
    }

    fn q_swap_dims(
        tensor: QuantizedTensor<Self>,
        dim1: usize,
        dim2: usize,
    ) -> QuantizedTensor<Self> {
        TensorStub
    }

    fn q_permute(tensor: QuantizedTensor<Self>, axes: &[usize]) -> QuantizedTensor<Self> {
        TensorStub
    }

    fn q_flip(tensor: QuantizedTensor<Self>, axes: &[usize]) -> QuantizedTensor<Self> {
        TensorStub
    }

    fn q_select(
        tensor: QuantizedTensor<Self>,
        dim: usize,
        indices: IntTensor<Self>,
    ) -> QuantizedTensor<Self> {
        TensorStub
    }

    fn q_slice(tensor: QuantizedTensor<Self>, ranges: &[Range<usize>]) -> QuantizedTensor<Self> {
        TensorStub
    }

    fn q_expand(tensor: QuantizedTensor<Self>, shape: Shape) -> QuantizedTensor<Self> {
        TensorStub
    }
}

impl TransactionOps<Self> for BackendStub {}

impl Backend for BackendStub {
    type Device = DeviceStub;
    type FloatTensorPrimitive = TensorStub;
    type FloatElem = f32;
    type IntTensorPrimitive = TensorStub;
    type IntElem = i32;
    type BoolTensorPrimitive = TensorStub;
    type BoolElem = bool;
    type QuantizedTensorPrimitive = TensorStub;
    type QuantizedEncoding = u32;

    fn name(device: &Self::Device) -> String {
        format!("BackendStub on device {:?}", device.id())
    }

    fn seed(seed: u64) {}
}
