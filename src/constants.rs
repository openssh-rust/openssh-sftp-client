macro_rules! def_constants {
    ( $name:ident, $val:literal ) => {
        pub const $name: u32 = $val;
    };
}

// version
def_constants!(SSH2_FILEXFER_VERSION, 3);

// client to server
def_constants!(SSH_FXP_INIT, 1);
def_constants!(SSH_FXP_OPEN, 3);
def_constants!(SSH_FXP_CLOSE, 4);
def_constants!(SSH_FXP_READ, 5);
def_constants!(SSH_FXP_WRITE, 6);
def_constants!(SSH_FXP_LSTAT, 7);
def_constants!(SSH_FXP_FSTAT, 8);
def_constants!(SSH_FXP_SETSTAT, 9);
def_constants!(SSH_FXP_FSETSTAT, 10);
def_constants!(SSH_FXP_OPENDIR, 11);
def_constants!(SSH_FXP_READDIR, 12);
def_constants!(SSH_FXP_REMOVE, 13);
def_constants!(SSH_FXP_MKDIR, 14);
def_constants!(SSH_FXP_RMDIR, 15);
def_constants!(SSH_FXP_REALPATH, 16);
def_constants!(SSH_FXP_STAT, 17);
def_constants!(SSH_FXP_RENAME, 18);
def_constants!(SSH_FXP_READLINK, 19);
def_constants!(SSH_FXP_SYMLINK, 20);

// server to client
def_constants!(SSH_FXP_VERSION, 2);
def_constants!(SSH_FXP_STATUS, 101);
def_constants!(SSH_FXP_HANDLE, 102);
def_constants!(SSH_FXP_DATA, 103);
def_constants!(SSH_FXP_NAME, 104);
def_constants!(SSH_FXP_ATTRS, 105);

def_constants!(SSH_FXP_EXTENDED, 200);
def_constants!(SSH_FXP_EXTENDED_REPLY, 201);

// status messages
def_constants!(SSH_FX_OK, 0);
def_constants!(SSH_FX_EOF, 1);
def_constants!(SSH_FX_NO_SUCH_FILE, 2);
def_constants!(SSH_FX_PERMISSION_DENIED, 3);
def_constants!(SSH_FX_FAILURE, 4);
def_constants!(SSH_FX_BAD_MESSAGE, 5);
def_constants!(SSH_FX_NO_CONNECTION, 6);
def_constants!(SSH_FX_CONNECTION_LOST, 7);
def_constants!(SSH_FX_OP_UNSUPPORTED, 8);

// attributes
def_constants!(SSH_FILEXFER_ATTR_SIZE, 0x00000001);
def_constants!(SSH_FILEXFER_ATTR_UIDGID, 0x00000002);
def_constants!(SSH_FILEXFER_ATTR_PERMISSIONS, 0x00000004);
def_constants!(SSH_FILEXFER_ATTR_ACMODTIME, 0x00000008);
def_constants!(SSH_FILEXFER_ATTR_EXTENDED, 0x80000000);

// open modes
def_constants!(SSH_FXF_READ, 0x00000001);
def_constants!(SSH_FXF_WRITE, 0x00000002);
def_constants!(SSH_FXF_APPEND, 0x00000004);
def_constants!(SSH_FXF_CREAT, 0x00000008);
def_constants!(SSH_FXF_TRUNC, 0x00000010);
def_constants!(SSH_FXF_EXCL, 0x00000020);
