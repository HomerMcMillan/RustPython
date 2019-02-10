use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objint;
use super::objtype;
use num_bigint::{BigInt, Sign};
use num_integer::Integer;
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::cell::Ref;
use std::ops::Mul;

#[derive(Debug, Clone)]
pub struct RangeType {
    // Unfortunately Rust's built in range type doesn't support things like indexing
    // or ranges where start > end so we need to roll our own.
    pub start: BigInt,
    pub end: BigInt,
    pub step: BigInt,
}

impl RangeType {
    #[inline]
    pub fn try_len(&self) -> Option<usize> {
        match self.step.sign() {
            Sign::Plus if self.start < self.end => ((&self.end - &self.start - 1usize)
                / &self.step)
                .to_usize()
                .map(|sz| sz + 1),
            Sign::Minus if self.start > self.end => ((&self.start - &self.end - 1usize)
                / (-&self.step))
                .to_usize()
                .map(|sz| sz + 1),
            _ => Some(0),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.try_len().unwrap()
    }

    #[inline]
    fn offset(&self, value: &BigInt) -> Option<BigInt> {
        match self.step.sign() {
            Sign::Plus if value >= &self.start && value < &self.end => Some(value - &self.start),
            Sign::Minus if value <= &self.start && value > &self.end => Some(&self.start - value),
            _ => None,
        }
    }

    #[inline]
    pub fn contains(&self, value: &BigInt) -> bool {
        match self.offset(value) {
            Some(ref offset) => offset.is_multiple_of(&self.step),
            None => false,
        }
    }

    #[inline]
    pub fn index_of(&self, value: &BigInt) -> Option<BigInt> {
        match self.offset(value) {
            Some(ref offset) if offset.is_multiple_of(&self.step) => {
                Some((offset / &self.step).abs())
            }
            Some(_) | None => None,
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        (self.start <= self.end && self.step.is_negative())
            || (self.start >= self.end && self.step.is_positive())
    }

    #[inline]
    pub fn forward(&self) -> bool {
        self.start < self.end
    }

    #[inline]
    pub fn get<'a, T>(&'a self, index: T) -> Option<BigInt>
    where
        &'a BigInt: Mul<T, Output = BigInt>,
    {
        let result = &self.start + &self.step * index;

        if (self.forward() && !self.is_empty() && result < self.end)
            || (!self.forward() && !self.is_empty() && result > self.end)
        {
            Some(result)
        } else {
            None
        }
    }

    #[inline]
    pub fn reversed(&self) -> Self {
        // compute the last element that is actually contained within the range
        // this is the new start
        let remainder = ((&self.end - &self.start) % &self.step).abs();
        let start = if remainder.is_zero() {
            &self.end - &self.step
        } else {
            &self.end - &remainder
        };

        match self.step.sign() {
            Sign::Plus => RangeType {
                start,
                end: &self.start - 1,
                step: -&self.step,
            },
            Sign::Minus => RangeType {
                start,
                end: &self.start + 1,
                step: -&self.step,
            },
            Sign::NoSign => unreachable!(),
        }
    }

    pub fn repr(&self) -> String {
        if self.step == BigInt::one() {
            format!("range({}, {})", self.start, self.end)
        } else {
            format!("range({}, {}, {})", self.start, self.end, self.step)
        }
    }
}

pub fn init(context: &PyContext) {
    let ref range_type = context.range_type;

    let range_doc = "range(stop) -> range object\n\
                     range(start, stop[, step]) -> range object\n\n\
                     Return an object that produces a sequence of integers from start (inclusive)\n\
                     to stop (exclusive) by step.  range(i, j) produces i, i+1, i+2, ..., j-1.\n\
                     start defaults to 0, and stop is omitted!  range(4) produces 0, 1, 2, 3.\n\
                     These are exactly the valid indices for a list of 4 elements.\n\
                     When step is given, it specifies the increment (or decrement).";

    context.set_attr(&range_type, "__new__", context.new_rustfunc(range_new));
    context.set_attr(&range_type, "__iter__", context.new_rustfunc(range_iter));
    context.set_attr(
        &range_type,
        "__reversed__",
        context.new_rustfunc(range_reversed),
    );
    context.set_attr(
        &range_type,
        "__doc__",
        context.new_str(range_doc.to_string()),
    );
    context.set_attr(&range_type, "__len__", context.new_rustfunc(range_len));
    context.set_attr(
        &range_type,
        "__getitem__",
        context.new_rustfunc(range_getitem),
    );
    context.set_attr(&range_type, "__repr__", context.new_rustfunc(range_repr));
    context.set_attr(&range_type, "__bool__", context.new_rustfunc(range_bool));
    context.set_attr(
        &range_type,
        "__contains__",
        context.new_rustfunc(range_contains),
    );
    context.set_attr(&range_type, "index", context.new_rustfunc(range_index));
}

fn range_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(cls, None), (first, Some(vm.ctx.int_type()))],
        optional = [
            (second, Some(vm.ctx.int_type())),
            (step, Some(vm.ctx.int_type()))
        ]
    );

    let start = if let Some(_) = second {
        objint::get_value(first)
    } else {
        BigInt::zero()
    };

    let end = if let Some(pyint) = second {
        objint::get_value(pyint)
    } else {
        objint::get_value(first)
    };

    let step = if let Some(pyint) = step {
        objint::get_value(pyint)
    } else {
        BigInt::one()
    };

    if step.is_zero() {
        Err(vm.new_value_error("range with 0 step size".to_string()))
    } else {
        Ok(PyObject::new(
            PyObjectPayload::Range {
                range: RangeType { start, end, step },
            },
            cls.clone(),
        ))
    }
}

fn range_iter(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(range, Some(vm.ctx.range_type()))]);

    Ok(PyObject::new(
        PyObjectPayload::Iterator {
            position: 0,
            iterated_obj: range.clone(),
        },
        vm.ctx.iter_type(),
    ))
}

fn range_reversed(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.range_type()))]);

    let range = get_range(zelf).reversed();
    Ok(PyObject::new(
        PyObjectPayload::Iterator {
            position: 0,
            iterated_obj: PyObject::new(PyObjectPayload::Range { range }, vm.ctx.range_type()),
        },
        vm.ctx.iter_type(),
    ))
}

fn range_len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.range_type()))]);

    if let Some(len) = get_range(zelf).try_len() {
        Ok(vm.ctx.new_int(len))
    } else {
        Err(vm.new_overflow_error("Python int too large to convert to Rust usize".to_string()))
    }
}

fn range_getitem(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.range_type())), (subscript, None)]
    );
    let range = get_range(zelf).clone();

    match subscript.borrow().payload {
        PyObjectPayload::Integer { ref value } => {
            if let Some(int) = range.get(value) {
                Ok(vm.ctx.new_int(int))
            } else {
                Err(vm.new_index_error("range object index out of range".to_string()))
            }
        }
        PyObjectPayload::Slice {
            ref start,
            ref stop,
            ref step,
        } => {
            let new_start = if let Some(int) = start {
                if let Some(i) = range.get(int) {
                    i
                } else {
                    range.start.clone()
                }
            } else {
                range.start.clone()
            };

            let new_end = if let Some(int) = stop {
                if let Some(i) = range.get(int) {
                    i
                } else {
                    range.end
                }
            } else {
                range.end
            };

            let new_step = if let Some(int) = step {
                int * range.step
            } else {
                range.step
            };

            Ok(PyObject::new(
                PyObjectPayload::Range {
                    range: RangeType {
                        start: new_start,
                        end: new_end,
                        step: new_step,
                    },
                },
                vm.ctx.range_type(),
            ))
        }

        _ => Err(vm.new_type_error("range indices must be integer or slice".to_string())),
    }
}

fn range_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.range_type()))]);
    Ok(vm.ctx.new_str(get_range(zelf).repr()))
}

fn range_bool(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.range_type()))]);
    Ok(vm.ctx.new_bool(!get_range(zelf).is_empty()))
}

fn range_contains(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.range_type())), (needle, None)]
    );
    let range = get_range(zelf);
    Ok(vm.ctx.new_bool(match needle.borrow().payload {
        PyObjectPayload::Integer { ref value } => range.contains(value),
        _ => false,
    }))
}

fn range_index(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.range_type())), (needle, None)]
    );
    let range = get_range(zelf);
    match needle.borrow().payload {
        PyObjectPayload::Integer { ref value } => match range.index_of(value) {
            Some(idx) => Ok(vm.ctx.new_int(idx)),
            None => Err(vm.new_value_error(format!("{} is not in range", value))),
        },
        _ => Err(vm.new_value_error("sequence.index(x): x not in sequence".to_string())),
    }
}

fn get_range<'a>(obj: &'a PyObjectRef) -> Ref<'a, RangeType> {
    Ref::map(obj.borrow(), |x| {
        if let PyObjectPayload::Range { ref range } = x.payload {
            range
        } else {
            panic!("Inner error getting rangetype")
        }
    })
}
