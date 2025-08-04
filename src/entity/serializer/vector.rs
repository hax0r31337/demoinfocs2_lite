use crate::entity::{serializer::EntityField, serializer::EntityMultiComponents};

macro_rules! vector {
    ($name:ident, $($v: ident),+) => {
        #[derive(Debug, Clone, Default)]
        #[repr(C)]
        pub struct $name {
            $(pub $v: f32),+
        }

        impl EntityField for $name {
            #[inline(always)]
            fn new() -> Self {
                Self {
                    $($v: 0.0),+
                }
            }
        }

        impl EntityMultiComponents<f32, {vector!(@count $($v),+)}> for $name {
            #[inline(always)]
            fn get_items(&mut self) -> &mut [f32; vector!(@count $($v),+)] {
                self.as_mut_array()
            }
        }

        impl $name {
            #[inline(always)]
            pub fn as_mut_array(&mut self) -> &mut [f32; vector!(@count $($v),+)] {
                unsafe { &mut *(self as *mut Self as *mut [f32; vector!(@count $($v),+)]) }
            }
        }
    };

    (@count $($v:ident),+) => {
        <[()]>::len(&[$(vector!(@replace $v)),+])
    };

    (@replace $v:ident) => { () };
}

vector!(Vector2, x, y);
vector!(Vector3, x, y, z);
vector!(QAngle, pitch, yaw, roll);
vector!(Vector4, x, y, z, w);
vector!(Transform6, x, y, z, qx, qy, qz);
