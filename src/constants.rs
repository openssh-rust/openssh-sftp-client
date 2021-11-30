macro_rules! def_packet_type {
    ( $name:ident, $val:literal ) => {
        pub(crate) const $name: u8 = $val;
    };
}

macro_rules! def_u32_constants {
    ( $name:ident, $val:literal ) => {
        pub(crate) const $name: u32 = $val;
    };
}

macro_rules! def_str_constants {
    ( $name:ident, $val:literal ) => {
        pub(crate) const $name: &'static str = $val;
    };
}

// version
pub(crate) const SSH2_FILEXFER_VERSION: u8 = 3;

// client to server
def_packet_type!(SSH_FXP_INIT, 1);
def_packet_type!(SSH_FXP_OPEN, 3);
def_packet_type!(SSH_FXP_CLOSE, 4);
def_packet_type!(SSH_FXP_READ, 5);
def_packet_type!(SSH_FXP_WRITE, 6);
def_packet_type!(SSH_FXP_LSTAT, 7);
def_packet_type!(SSH_FXP_FSTAT, 8);
def_packet_type!(SSH_FXP_SETSTAT, 9);
def_packet_type!(SSH_FXP_FSETSTAT, 10);
def_packet_type!(SSH_FXP_OPENDIR, 11);
def_packet_type!(SSH_FXP_READDIR, 12);
def_packet_type!(SSH_FXP_REMOVE, 13);
def_packet_type!(SSH_FXP_MKDIR, 14);
def_packet_type!(SSH_FXP_RMDIR, 15);
def_packet_type!(SSH_FXP_REALPATH, 16);
def_packet_type!(SSH_FXP_STAT, 17);
def_packet_type!(SSH_FXP_RENAME, 18);
def_packet_type!(SSH_FXP_READLINK, 19);
def_packet_type!(SSH_FXP_SYMLINK, 20);

// server to client
def_packet_type!(SSH_FXP_VERSION, 2);
def_packet_type!(SSH_FXP_STATUS, 101);
def_packet_type!(SSH_FXP_HANDLE, 102);
def_packet_type!(SSH_FXP_DATA, 103);
def_packet_type!(SSH_FXP_NAME, 104);
def_packet_type!(SSH_FXP_ATTRS, 105);

def_packet_type!(SSH_FXP_EXTENDED, 200);
def_packet_type!(SSH_FXP_EXTENDED_REPLY, 201);

// status code
def_u32_constants!(SSH_FX_OK, 0);
def_u32_constants!(SSH_FX_EOF, 1);
def_u32_constants!(SSH_FX_NO_SUCH_FILE, 2);
def_u32_constants!(SSH_FX_PERMISSION_DENIED, 3);
def_u32_constants!(SSH_FX_FAILURE, 4);
def_u32_constants!(SSH_FX_BAD_MESSAGE, 5);
def_u32_constants!(SSH_FX_NO_CONNECTION, 6);
def_u32_constants!(SSH_FX_CONNECTION_LOST, 7);
def_u32_constants!(SSH_FX_OP_UNSUPPORTED, 8);

// attributes
def_u32_constants!(SSH_FILEXFER_ATTR_SIZE, 0x00000001);
def_u32_constants!(SSH_FILEXFER_ATTR_UIDGID, 0x00000002);
def_u32_constants!(SSH_FILEXFER_ATTR_PERMISSIONS, 0x00000004);
def_u32_constants!(SSH_FILEXFER_ATTR_ACMODTIME, 0x00000008);
def_u32_constants!(SSH_FILEXFER_ATTR_EXTENDED, 0x80000000);

// open modes
def_u32_constants!(SSH_FXF_READ, 0x00000001);
def_u32_constants!(SSH_FXF_WRITE, 0x00000002);
def_u32_constants!(SSH_FXF_APPEND, 0x00000004);
def_u32_constants!(SSH_FXF_CREAT, 0x00000008);
def_u32_constants!(SSH_FXF_TRUNC, 0x00000010);
def_u32_constants!(SSH_FXF_EXCL, 0x00000020);

// extensions
def_u32_constants!(SFTP_EXT_POSIX_RENAME, 0x00000001);
def_u32_constants!(SFTP_EXT_STATVFS, 0x00000002);
def_u32_constants!(SFTP_EXT_FSTATVFS, 0x00000004);
def_u32_constants!(SFTP_EXT_HARDLINK, 0x00000008);
def_u32_constants!(SFTP_EXT_FSYNC, 0x00000010);
def_u32_constants!(SFTP_EXT_LSETSTAT, 0x00000020);
def_u32_constants!(SFTP_EXT_LIMITS, 0x00000040);
def_u32_constants!(SFTP_EXT_PATH_EXPAND, 0x00000080);

// extension names
def_str_constants!(EXT_NAME_POSIX_RENAME, "posix-rename@openssh.com");
def_str_constants!(EXT_NAME_STATVFS, "statvfs@openssh.com");
def_str_constants!(EXT_NAME_FSTATVFS, "fstatvfs@openssh.com");
def_str_constants!(EXT_NAME_HARDLINK, "hardlink@openssh.com");
def_str_constants!(EXT_NAME_FSYNC, "fsync@openssh.com");
def_str_constants!(EXT_NAME_LSETSTAT, "lsetstat@openssh.com");
def_str_constants!(EXT_NAME_LIMITS, "limits@openssh.com");
def_str_constants!(EXT_NAME_EXPAND_PATH, "expand-path@openssh.com");
